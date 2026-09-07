//! Login inventory recognition follows Reliquary's record types, but searches
//! every repeated protobuf field instead of command IDs or outer field tags.
//! Inner records are validated against bundled public data before acceptance.
use std::collections::{BTreeMap, BTreeSet};

use protobuf::Message;
use serde::Deserialize;

use super::{
    proto::{Avatar::Avatar, AvatarPathData::AvatarPathData, Equipment::Equipment, Relic::Relic},
    protocol::{containers, parse_message, WireValue},
    HSR_CAPTURE_REVISION,
};
use crate::{model::*, reference::ReferenceCache, HsrError, HsrResult, LocalizedText};

#[derive(Deserialize)]
struct MainAffix {
    group: u32,
    id: u32,
    property: String,
}
#[derive(Deserialize)]
struct SubAffix {
    group: u32,
    id: u32,
    property: String,
    base: f64,
    step: f64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PacketReferences {
    main: Vec<MainAffix>,
    sub: Vec<SubAffix>,
    base_avatars: BTreeMap<u32, u32>,
}

pub struct InventoryDecoder {
    references: ReferenceCache,
    affixes: PacketReferences,
    pub characters: Option<Vec<ObservedCharacter>>,
    pub light_cones: Option<Vec<ObservedLightCone>>,
    pub relics: Option<Vec<ObservedGear>>,
}

impl InventoryDecoder {
    pub fn new(references: ReferenceCache) -> HsrResult<Self> {
        let affixes = serde_json::from_str(include_str!("../../assets/packet_affixes.json"))
            .map_err(|error| invalid(format!("bundled packet affixes: {error}")))?;
        Ok(Self {
            references,
            affixes,
            characters: None,
            light_cones: None,
            relics: None,
        })
    }

    pub fn reset(&mut self) {
        self.characters = None;
        self.light_cones = None;
        self.relics = None;
    }

    pub fn complete(&self) -> bool {
        self.characters.is_some() && self.light_cones.is_some() && self.relics.is_some()
    }

    pub fn snapshot(&self) -> Option<ObservationSnapshot> {
        Some(ObservationSnapshot {
            schema_version: OBSERVATION_SCHEMA_VERSION,
            evidence: ObservationEvidence {
                kind: EvidenceKind::PacketCapture,
                revision: HSR_CAPTURE_REVISION.to_owned(),
                coverage: InventoryCoverage {
                    characters: CoverageLevel::Complete,
                    light_cones: CoverageLevel::Complete,
                    relics: CoverageLevel::Complete,
                },
            },
            characters: self.characters.clone()?,
            light_cones: self.light_cones.clone()?,
            gear: self.relics.clone()?,
        })
    }

    pub fn receive(&mut self, bytes: &[u8]) -> HsrResult<()> {
        if self.complete() {
            return Ok(());
        }
        for container in containers(bytes) {
            let mut groups: BTreeMap<u32, Vec<&[u8]>> = BTreeMap::new();
            let fields = parse_message(container).unwrap_or_default();
            // GetAvatarDataScRsp carries is_get_all=true; AvatarSync does not.
            // Infer the boolean's position, not its rotating field number.
            let has_get_all = fields
                .iter()
                .any(|field| matches!(field.value, WireValue::Varint(1)));
            for field in fields {
                if let WireValue::Bytes(value) = field.value {
                    groups.entry(field.number).or_default().push(value);
                }
            }
            // A login character response contains both base progression and
            // path records. Do not mistake a single-character update for it.
            let bases = unique_group(&groups, |bytes| {
                let value = Avatar::parse_from_bytes(bytes).ok()?;
                (self.references.character(value.base_avatar_id).is_some()
                    && (1..=80).contains(&value.level)
                    && value.promotion <= 6
                    && value.first_met_time_stamp >= 1_600_000_000)
                    .then_some(value)
            });
            let paths = unique_group(&groups, |bytes| {
                let value = AvatarPathData::parse_from_bytes(bytes).ok()?;
                (self.references.character(value.avatar_id).is_some()
                    && value.rank <= 6
                    && !value.avatar_path_skill_tree.is_empty()
                    && value
                        .avatar_path_skill_tree
                        .iter()
                        .all(|s| s.point_id > 0 && s.level <= 20))
                .then_some(value)
            });
            if self.characters.is_none() && has_get_all {
                if let (Some((_, bases)), Some((_, paths))) = (bases, paths) {
                    let bases: BTreeMap<_, _> =
                        bases.into_iter().map(|b| (b.base_avatar_id, b)).collect();
                    let mut characters = Vec::new();
                    let mut seen = BTreeSet::new();
                    for path in paths {
                        if !seen.insert(path.avatar_id) {
                            return Err(invalid("duplicate character path"));
                        }
                        let base_id = self
                            .affixes
                            .base_avatars
                            .get(&path.avatar_id)
                            .copied()
                            .unwrap_or(path.avatar_id);
                        let base = bases.get(&base_id).ok_or_else(|| {
                            invalid(format!(
                                "character {} has no base progression",
                                path.avatar_id
                            ))
                        })?;
                        characters.push(ObservedCharacter {
                            character_id: path.avatar_id,
                            level: base.level as u8,
                            ascension: base.promotion as u8,
                            eidolon: path.rank as u8,
                        });
                    }
                    characters.sort_by_key(|c| c.character_id);
                    self.characters = Some(characters);
                }
            }
            // Require both inventory record families in the same container.
            // Missing collections are never reported as captured empty data.
            if self.light_cones.is_none() || self.relics.is_none() {
                let cones = unique_group(&groups, |bytes| {
                    let value = Equipment::parse_from_bytes(bytes).ok()?;
                    (self.references.light_cone(value.tid).is_some()
                        && value.unique_id > 0
                        && (1..=80).contains(&value.level)
                        && (1..=5).contains(&value.rank)
                        && value.promotion <= 6)
                        .then_some(value)
                });
                let relics = unique_group(&groups, |bytes| {
                    let value = Relic::parse_from_bytes(bytes).ok()?;
                    (self.references.gear(value.tid).is_some()
                        && value.unique_id > 0
                        && value.main_affix_id > 0
                        && value.level <= 15
                        && value.sub_affix_list.len() <= 4)
                        .then_some(value)
                });
                if let (Some((cone_tag, cones)), Some((relic_tag, relics))) = (cones, relics) {
                    if cone_tag == relic_tag {
                        continue;
                    }
                    let mut ids = BTreeSet::new();
                    let mut normalized_cones = Vec::new();
                    for cone in cones {
                        if !ids.insert(cone.unique_id) {
                            return Err(invalid("duplicate light cone instance"));
                        }
                        normalized_cones.push(ObservedLightCone {
                            light_cone_id: cone.tid,
                            level: cone.level as u8,
                            ascension: cone.promotion as u8,
                            superimposition: cone.rank as u8,
                            equipped_character_id: self.location(cone.equip_avatar_id)?,
                            lock: Some(cone.is_protected),
                        });
                    }
                    ids.clear();
                    let mut normalized_relics = Vec::new();
                    for relic in relics {
                        if !ids.insert(relic.unique_id) {
                            return Err(invalid("duplicate relic instance"));
                        }
                        normalized_relics.push(self.relic(relic)?);
                    }
                    self.light_cones = Some(normalized_cones);
                    self.relics = Some(normalized_relics);
                }
            }
        }
        Ok(())
    }

    fn location(&self, id: u32) -> HsrResult<Option<u32>> {
        if id == 0 {
            return Ok(None);
        }
        self.references
            .character(id)
            .map(|_| Some(id))
            .ok_or_else(|| invalid(format!("unknown equipped character {id}")))
    }

    fn relic(&self, relic: Relic) -> HsrResult<ObservedGear> {
        let piece = self
            .references
            .gear(relic.tid)
            .ok_or_else(|| invalid("unknown relic"))?;
        let main = self
            .affixes
            .main
            .iter()
            .find(|a| a.group == piece.main_affix_group && a.id == relic.main_affix_id)
            .ok_or_else(|| invalid(format!("unknown main affix for relic {}", relic.tid)))?;
        let main_value = self
            .references
            .relic_main_stat_value_for_piece(piece, &main.property, relic.level as u8)
            .ok_or_else(|| invalid("main affix progression unavailable"))?;
        let mut substats = Vec::new();
        for sub in relic.sub_affix_list {
            let affix = self
                .affixes
                .sub
                .iter()
                .find(|a| a.group == u32::from(piece.rarity) && a.id == sub.affix_id)
                .ok_or_else(|| invalid(format!("unknown sub affix {}", sub.affix_id)))?;
            if sub.cnt == 0 || sub.cnt > 6 || sub.step > sub.cnt * 2 {
                return Err(invalid("invalid substat roll counts"));
            }
            let stat = self
                .references
                .stat(&affix.property)
                .ok_or_else(|| invalid("unknown substat property"))?;
            let factor = match stat.value_kind {
                StatValueKind::Ratio => 100.0,
                StatValueKind::Flat => 1.0,
                StatValueKind::Unknown => return Err(invalid("unknown substat units")),
            };
            substats.push(ObservedSubstat {
                stat_key: affix.property.clone(),
                value: (f64::from(sub.cnt) * affix.base + f64::from(sub.step) * affix.step)
                    * factor,
            });
        }
        Ok(ObservedGear {
            piece_id: relic.tid,
            level: relic.level as u8,
            main_stat_key: main.property.clone(),
            main_stat_value: main_value,
            substats,
            equipped_character_id: self.location(relic.equip_avatar_id)?,
            lock: Some(relic.is_protected),
            discard: Some(relic.is_discarded),
        })
    }
}

fn unique_group<'a, T>(
    groups: &BTreeMap<u32, Vec<&'a [u8]>>,
    decode: impl Fn(&'a [u8]) -> Option<T>,
) -> Option<(u32, Vec<T>)> {
    let mut found = None;
    for (&tag, records) in groups {
        if let Some(values) = records
            .iter()
            .map(|record| decode(record))
            .collect::<Option<Vec<_>>>()
        {
            if found.is_some() {
                return None;
            }
            found = Some((tag, values));
        }
    }
    found
}

fn invalid(detail: impl Into<String>) -> HsrError {
    HsrError::new("HSR-CAPTURE-INVENTORY", LocalizedText::new(
        "无法识别完整的星穹铁道库存。请更新 GOODCapture 后重新登录抓包。",
        "The complete Star Rail inventory could not be decoded. Update GOODCapture and capture a new login."), detail)
}
