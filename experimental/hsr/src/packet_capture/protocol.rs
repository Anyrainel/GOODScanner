//! Privacy-safe decoding of decrypted HSR achievement protobuf payloads.
//!
//! Command IDs and protobuf field numbers rotate between game versions. The
//! decoder therefore identifies the repeated achievement records from their
//! structure and the complete public achievement-ID set supplied by GIlore.
//! It deliberately does not retain raw statuses, completion timestamps, or
//! packet bytes.

use std::collections::{BTreeMap, BTreeSet};

use crate::{localization::LocalizedText, HsrError, HsrResult};

use super::DecodedAchievementSnapshot;

const MIN_ACHIEVEMENT_RECORDS: usize = 5;
const MIN_KNOWN_ID_MATCHES: usize = 5;
const MIN_UNIQUE_ID_PERCENT: usize = 80;
const MIN_KNOWN_ID_PERCENT: usize = 80;
const MAX_RECORD_FIELDS: usize = 8;
const MAX_PROTOBUF_FIELD_NUMBER: u64 = (1 << 29) - 1;

const REFERENCE_HINT: LocalizedText = LocalizedText::new(
    "抓包中出现尚未收录的成就。请刷新游戏数据后重新抓包。",
    "Capture found an achievement missing from the cached game data. Refresh game data and capture again.",
);
const REFERENCE_SET_HINT: LocalizedText = LocalizedText::new(
    "星穹铁道成就数据缺失或无效。请刷新游戏数据后重试。",
    "Star Rail achievement data is missing or invalid. Refresh game data and retry.",
);

/// Decode one decrypted HSR command payload.
///
/// `Ok(None)` means this command cannot be proven to be the complete
/// achievement response. `Ok(Some(_))` can contain zero IDs: that is a real,
/// present achievement snapshot in which no record has completed status.
pub fn decode_achievement_command(
    proto_data: &[u8],
    known_achievement_ids: &BTreeSet<u32>,
) -> HsrResult<Option<DecodedAchievementSnapshot>> {
    validate_known_ids(known_achievement_ids)?;

    let Some(fields) = parse_message(proto_data) else {
        return Ok(None);
    };

    let mut groups: BTreeMap<u32, Vec<&[u8]>> = BTreeMap::new();
    for field in fields {
        let WireValue::Bytes(bytes) = field.value else {
            continue;
        };
        groups.entry(field.number).or_default().push(bytes);
    }

    let mut decoded = None;
    for entries in groups.values() {
        // Login GetQuestData-style responses mix public achievements with
        // quests in one repeated field. Skip entries that are not scalar
        // records instead of rejecting the whole group.
        let records = entries
            .iter()
            .filter_map(|bytes| parse_scalar_record(bytes))
            .collect::<Vec<_>>();
        let Some(candidate) = decode_candidate(&records, known_achievement_ids)? else {
            continue;
        };

        // More than one independently plausible repeated record group is
        // ambiguous. Refuse it instead of risking an authoritative empty or
        // partial replacement in the consumer.
        if decoded.is_some() {
            return Ok(None);
        }
        decoded = Some(candidate);
    }

    Ok(decoded)
}

pub(super) fn collect_known_ids<I>(known_ids: I) -> HsrResult<BTreeSet<u32>>
where
    I: IntoIterator<Item = u32>,
{
    let mut result = BTreeSet::new();
    for id in known_ids {
        if id == 0 {
            return Err(HsrError::new(
                "HSR-ACHIEVEMENT-REFERENCE-ID",
                REFERENCE_SET_HINT,
                "the public achievement reference contains ID 0",
            ));
        }
        if !result.insert(id) {
            return Err(HsrError::new(
                "HSR-ACHIEVEMENT-REFERENCE-ID",
                REFERENCE_SET_HINT,
                format!("the public achievement reference repeats ID {id}"),
            ));
        }
    }
    validate_known_ids(&result)?;
    Ok(result)
}

fn validate_known_ids(known_ids: &BTreeSet<u32>) -> HsrResult<()> {
    if known_ids.is_empty() {
        return Err(HsrError::new(
            "HSR-ACHIEVEMENT-REFERENCE-EMPTY",
            REFERENCE_SET_HINT,
            "the public achievement reference ID set is empty",
        ));
    }
    if known_ids.contains(&0) {
        return Err(HsrError::new(
            "HSR-ACHIEVEMENT-REFERENCE-ID",
            REFERENCE_SET_HINT,
            "the public achievement reference contains ID 0",
        ));
    }
    Ok(())
}

fn decode_candidate(
    records: &[BTreeMap<u32, u64>],
    known_ids: &BTreeSet<u32>,
) -> HsrResult<Option<DecodedAchievementSnapshot>> {
    if records.len() < MIN_ACHIEVEMENT_RECORDS {
        return Ok(None);
    }

    let Some(id_tag) = infer_id_tag(records, known_ids) else {
        return Ok(None);
    };
    let Some(status_tag) = infer_status_tag(records, id_tag) else {
        return Ok(None);
    };

    let mut unknown_ids = BTreeSet::new();
    let mut completed_ids = Vec::new();
    let mut known_records = 0usize;
    for record in records {
        let Some(&raw_id) = record.get(&id_tag) else {
            continue;
        };
        let Ok(id) = u32::try_from(raw_id) else {
            unknown_ids.insert(raw_id);
            continue;
        };
        if id == 0 || !known_ids.contains(&id) {
            unknown_ids.insert(raw_id);
            continue;
        }

        let Some(&status) = record.get(&status_tag) else {
            return Ok(None);
        };
        known_records += 1;
        if matches!(status, 2 | 3) {
            completed_ids.push(id);
        }
    }

    if known_records < MIN_ACHIEVEMENT_RECORDS {
        return Ok(None);
    }

    // A homogeneous achievement list with unknown IDs means GIlore is stale.
    // A mixed quest/achievement list is expected on live 4.5 login; keep only
    // the public IDs.
    if !unknown_ids.is_empty() && known_records * 100 >= records.len() * MIN_KNOWN_ID_PERCENT {
        let rendered = unknown_ids
            .iter()
            .take(16)
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let omitted = unknown_ids.len().saturating_sub(16);
        let suffix = if omitted == 0 {
            String::new()
        } else {
            format!(" (and {omitted} more)")
        };
        return Err(HsrError::new(
            "HSR-ACHIEVEMENT-REFERENCE-STALE",
            REFERENCE_HINT,
            format!("unknown public achievement IDs: {rendered}{suffix}"),
        ));
    }

    completed_ids.sort_unstable();
    completed_ids.dedup();
    Ok(Some(DecodedAchievementSnapshot::new(completed_ids)))
}

fn infer_id_tag(records: &[BTreeMap<u32, u64>], known_ids: &BTreeSet<u32>) -> Option<u32> {
    let tags = records
        .iter()
        .flat_map(|record| record.keys().copied())
        .collect::<BTreeSet<_>>();
    let mut candidates = Vec::new();

    for tag in tags {
        let values = records
            .iter()
            .map(|record| record.get(&tag).copied())
            .collect::<Option<Vec<_>>>();
        let Some(values) = values else {
            continue;
        };

        let known_matches = values
            .iter()
            .filter(|&&value| {
                u32::try_from(value)
                    .ok()
                    .is_some_and(|id| known_ids.contains(&id))
            })
            .count();
        let unique = values.iter().copied().collect::<BTreeSet<_>>().len();
        // Do not require known IDs to dominate the mixed quest list; uniqueness
        // plus a handful of GIlore matches is enough to locate the ID tag.
        if known_matches < MIN_KNOWN_ID_MATCHES
            || unique * 100 < records.len() * MIN_UNIQUE_ID_PERCENT
        {
            continue;
        }
        candidates.push((known_matches, unique, tag));
    }

    candidates.sort_unstable_by(|left, right| right.cmp(left));
    let best = candidates.first().copied()?;
    if candidates
        .get(1)
        .is_some_and(|next| next.0 == best.0 && next.1 == best.1)
    {
        return None;
    }
    Some(best.2)
}

fn infer_status_tag(records: &[BTreeMap<u32, u64>], id_tag: u32) -> Option<u32> {
    let tags = records
        .iter()
        .flat_map(|record| record.keys().copied())
        .filter(|&tag| tag != id_tag)
        .collect::<BTreeSet<_>>();
    let mut candidates = Vec::new();

    for tag in tags {
        let values = records
            .iter()
            .map(|record| record.get(&tag).copied())
            .collect::<Option<Vec<_>>>();
        let Some(values) = values else {
            continue;
        };
        if values.iter().any(|&value| value > 4) {
            continue;
        }

        let completed = values
            .iter()
            .filter(|&&value| matches!(value, 2 | 3))
            .count();
        let distinct = values.iter().copied().collect::<BTreeSet<_>>().len();
        candidates.push((completed, distinct, tag));
    }

    candidates.sort_unstable_by(|left, right| right.cmp(left));
    let best = candidates.first().copied()?;
    if candidates
        .get(1)
        .is_some_and(|next| next.0 == best.0 && next.1 == best.1)
    {
        return None;
    }
    Some(best.2)
}

fn parse_scalar_record(bytes: &[u8]) -> Option<BTreeMap<u32, u64>> {
    let fields = parse_message(bytes)?;
    if !(2..=MAX_RECORD_FIELDS).contains(&fields.len()) {
        return None;
    }

    let mut record = BTreeMap::new();
    for field in fields {
        let value = match field.value {
            WireValue::Varint(value) => value,
            // Quest also contains packed progress counters. They are not
            // candidates for the achievement ID or status field.
            WireValue::Bytes(bytes) => {
                let mut cursor = 0;
                while cursor < bytes.len() {
                    read_varint(bytes, &mut cursor)?;
                }
                continue;
            },
            _ => return None,
        };
        if record.insert(field.number, value).is_some() {
            return None;
        }
    }
    Some(record)
}

/// Token responses contain one 64-bit seed alongside public-size integers
/// and strings. Infer its tag only during dispatch decryption; never use a
/// command ID or rotate the session key on later gameplay packets.
pub(crate) fn infer_session_seed(bytes: &[u8]) -> Option<u64> {
    let fields = parse_message(bytes)?;
    if fields.len() > 8 {
        return None;
    }
    let mut seeds = fields.iter().filter_map(|field| match field.value {
        WireValue::Varint(value) if value > u32::MAX as u64 => Some(value),
        _ => None,
    });
    let seed = seeds.next()?;
    seeds.next().is_none().then_some(seed)
}

/// Walk well-formed protobuf wrappers, bounded by depth and total input size.
/// Each candidate stays intact so a rejected entry cannot silently disappear.
pub(crate) fn containers(bytes: &[u8]) -> Vec<&[u8]> {
    fn visit<'a>(bytes: &'a [u8], depth: usize, out: &mut Vec<&'a [u8]>) {
        if depth > 4 || out.len() >= 4096 {
            return;
        }
        let Some(fields) = parse_message(bytes) else {
            return;
        };
        out.push(bytes);
        for field in fields {
            if let WireValue::Bytes(child) = field.value {
                visit(child, depth + 1, out);
            }
        }
    }
    let mut result = Vec::new();
    if bytes.len() <= 16 * 1024 * 1024 {
        visit(bytes, 0, &mut result);
    }
    result
}

#[derive(Clone, Copy)]
pub(crate) struct WireField<'a> {
    pub number: u32,
    pub value: WireValue<'a>,
}

#[derive(Clone, Copy)]
pub(crate) enum WireValue<'a> {
    Varint(u64),
    Fixed64,
    Bytes(&'a [u8]),
    Fixed32,
}

pub(crate) fn parse_message(bytes: &[u8]) -> Option<Vec<WireField<'_>>> {
    let mut cursor = 0usize;
    let mut fields = Vec::new();
    while cursor < bytes.len() {
        let key = read_varint(bytes, &mut cursor)?;
        let field_number = key >> 3;
        let wire_type = key & 0x07;
        if field_number == 0 || field_number > MAX_PROTOBUF_FIELD_NUMBER {
            return None;
        }
        let number = u32::try_from(field_number).ok()?;

        let value = match wire_type {
            0 => WireValue::Varint(read_varint(bytes, &mut cursor)?),
            1 => {
                cursor = cursor.checked_add(8)?;
                if cursor > bytes.len() {
                    return None;
                }
                WireValue::Fixed64
            },
            2 => {
                let length = usize::try_from(read_varint(bytes, &mut cursor)?).ok()?;
                let end = cursor.checked_add(length)?;
                let value = bytes.get(cursor..end)?;
                cursor = end;
                WireValue::Bytes(value)
            },
            // Deprecated protobuf groups are not used by this protocol. A
            // group would make structural inference ambiguous, so reject it.
            3 | 4 => return None,
            5 => {
                cursor = cursor.checked_add(4)?;
                if cursor > bytes.len() {
                    return None;
                }
                WireValue::Fixed32
            },
            _ => return None,
        };
        fields.push(WireField { number, value });
    }
    Some(fields)
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_ids_reject_empty_zero_and_duplicates() {
        assert!(collect_known_ids([]).is_err());
        assert!(collect_known_ids([4_010_101, 0]).is_err());
        assert!(collect_known_ids([4_010_101, 4_010_101]).is_err());
    }

    #[test]
    fn malformed_varints_and_lengths_are_not_packets() {
        let known = BTreeSet::from([4_010_101, 4_010_102, 4_010_103, 4_010_104, 4_010_105]);
        assert_eq!(decode_achievement_command(&[0x80], &known).unwrap(), None);
        assert_eq!(
            decode_achievement_command(&[0x0a, 0xff, 0xff], &known).unwrap(),
            None
        );
    }

    #[test]
    fn mixed_quest_records_keep_only_known_completed_achievements() {
        let known = BTreeSet::from([
            4_010_101, 4_010_102, 4_010_103, 4_010_104, 4_010_105, 4_010_106,
        ]);
        let mut records = Vec::new();
        for id in &known {
            records.push(scalar_record(11, *id, 12, 3));
        }
        for quest_id in [
            1_001_711u32,
            2_202_001,
            5_001_001,
            6_001_001,
            7_301_001,
            3_002_020,
        ] {
            records.push(scalar_record(11, quest_id, 12, 1));
        }
        // A nested junk entry in the same repeated field must not drop the group.
        records.push(vec![0x0a, 0x03, b'b', b'a', b'd']);
        let decoded = decode_achievement_command(&repeated(13, &records), &known)
            .unwrap()
            .expect("mixed login list must still be recognized");
        assert_eq!(
            decoded.completed_ids(),
            &[4_010_101, 4_010_102, 4_010_103, 4_010_104, 4_010_105, 4_010_106]
        );
    }

    fn put_varint(out: &mut Vec<u8>, mut value: u64) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn scalar_record(id_tag: u32, id: u32, status_tag: u32, status: u32) -> Vec<u8> {
        let mut out = Vec::new();
        put_varint(&mut out, u64::from(id_tag) << 3);
        put_varint(&mut out, u64::from(id));
        put_varint(&mut out, u64::from(status_tag) << 3);
        put_varint(&mut out, u64::from(status));
        out
    }

    fn repeated(field: u32, records: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for record in records {
            put_varint(&mut out, (u64::from(field) << 3) | 2);
            put_varint(&mut out, record.len() as u64);
            out.extend_from_slice(record);
        }
        out
    }
}
