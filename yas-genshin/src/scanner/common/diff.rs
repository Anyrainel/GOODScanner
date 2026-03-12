use std::collections::HashMap;
use std::fmt;

use super::models::*;

/// A single field-level difference between expected and actual values.
#[derive(Debug)]
pub struct FieldDiff {
    pub field: String,
    pub expected: String,
    pub actual: String,
}

impl fmt::Display for FieldDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "  {} : expected={}, actual={}", self.field, self.expected, self.actual)
    }
}

/// Diff for a single character.
#[derive(Debug)]
pub struct CharacterDiff {
    pub key: String,
    pub status: DiffStatus,
    pub field_diffs: Vec<FieldDiff>,
}

/// Diff for a single weapon.
#[derive(Debug)]
pub struct WeaponDiff {
    pub index: usize,
    pub key_expected: String,
    pub key_actual: String,
    pub status: DiffStatus,
    pub field_diffs: Vec<FieldDiff>,
}

/// Diff for a single artifact.
#[derive(Debug)]
pub struct ArtifactDiff {
    pub index: usize,
    pub set_expected: String,
    pub set_actual: String,
    pub status: DiffStatus,
    pub field_diffs: Vec<FieldDiff>,
}

#[derive(Debug, PartialEq)]
pub enum DiffStatus {
    /// Both sides present and compared
    Compared,
    /// Present in expected but missing from actual
    Missing,
    /// Present in actual but not in expected
    Extra,
}

/// Summary statistics for a diff.
#[derive(Debug, Default)]
pub struct DiffSummary {
    pub characters_matched: usize,
    pub characters_mismatched: usize,
    pub characters_missing: usize,
    pub characters_extra: usize,
    pub weapons_matched: usize,
    pub weapons_mismatched: usize,
    pub weapons_missing: usize,
    pub weapons_extra: usize,
    pub artifacts_matched: usize,
    pub artifacts_mismatched: usize,
    pub artifacts_missing: usize,
    pub artifacts_extra: usize,
}

impl DiffSummary {
    pub fn total_errors(&self) -> usize {
        self.characters_mismatched + self.characters_missing + self.characters_extra
            + self.weapons_mismatched + self.weapons_missing + self.weapons_extra
            + self.artifacts_mismatched + self.artifacts_missing + self.artifacts_extra
    }
}

/// Full diff result between two GOOD exports.
#[derive(Debug)]
pub struct DiffResult {
    pub character_diffs: Vec<CharacterDiff>,
    pub weapon_diffs: Vec<WeaponDiff>,
    pub artifact_diffs: Vec<ArtifactDiff>,
    pub summary: DiffSummary,
}

/// Compare two GOOD exports and return field-level diffs.
pub fn diff_exports(actual: &GoodExport, expected: &GoodExport) -> DiffResult {
    let mut summary = DiffSummary::default();

    let character_diffs = diff_characters(
        actual.characters.as_deref().unwrap_or(&[]),
        expected.characters.as_deref().unwrap_or(&[]),
        &mut summary,
    );

    let weapon_diffs = diff_weapons(
        actual.weapons.as_deref().unwrap_or(&[]),
        expected.weapons.as_deref().unwrap_or(&[]),
        &mut summary,
    );

    let artifact_diffs = diff_artifacts(
        actual.artifacts.as_deref().unwrap_or(&[]),
        expected.artifacts.as_deref().unwrap_or(&[]),
        &mut summary,
    );

    DiffResult {
        character_diffs,
        weapon_diffs,
        artifact_diffs,
        summary,
    }
}

/// Compare characters by key (name).
fn diff_characters(
    actual: &[GoodCharacter],
    expected: &[GoodCharacter],
    summary: &mut DiffSummary,
) -> Vec<CharacterDiff> {
    let mut diffs = Vec::new();

    // Build lookup map for actual characters
    let actual_map: HashMap<&str, &GoodCharacter> = actual
        .iter()
        .map(|c| (c.key.as_str(), c))
        .collect();

    let expected_map: HashMap<&str, &GoodCharacter> = expected
        .iter()
        .map(|c| (c.key.as_str(), c))
        .collect();

    // Check expected characters
    for exp in expected {
        if let Some(act) = actual_map.get(exp.key.as_str()) {
            let field_diffs = diff_character_fields(act, exp);
            if field_diffs.is_empty() {
                summary.characters_matched += 1;
            } else {
                summary.characters_mismatched += 1;
                diffs.push(CharacterDiff {
                    key: exp.key.clone(),
                    status: DiffStatus::Compared,
                    field_diffs,
                });
            }
        } else {
            summary.characters_missing += 1;
            diffs.push(CharacterDiff {
                key: exp.key.clone(),
                status: DiffStatus::Missing,
                field_diffs: Vec::new(),
            });
        }
    }

    // Check for extra characters in actual
    for act in actual {
        if !expected_map.contains_key(act.key.as_str()) {
            summary.characters_extra += 1;
            diffs.push(CharacterDiff {
                key: act.key.clone(),
                status: DiffStatus::Extra,
                field_diffs: Vec::new(),
            });
        }
    }

    diffs
}

fn diff_character_fields(actual: &GoodCharacter, expected: &GoodCharacter) -> Vec<FieldDiff> {
    let mut diffs = Vec::new();

    if actual.level != expected.level {
        diffs.push(FieldDiff {
            field: "level".into(),
            expected: expected.level.to_string(),
            actual: actual.level.to_string(),
        });
    }
    if actual.constellation != expected.constellation {
        diffs.push(FieldDiff {
            field: "constellation".into(),
            expected: expected.constellation.to_string(),
            actual: actual.constellation.to_string(),
        });
    }
    if actual.ascension != expected.ascension {
        diffs.push(FieldDiff {
            field: "ascension".into(),
            expected: expected.ascension.to_string(),
            actual: actual.ascension.to_string(),
        });
    }
    if actual.talent.auto != expected.talent.auto {
        diffs.push(FieldDiff {
            field: "talent.auto".into(),
            expected: expected.talent.auto.to_string(),
            actual: actual.talent.auto.to_string(),
        });
    }
    if actual.talent.skill != expected.talent.skill {
        diffs.push(FieldDiff {
            field: "talent.skill".into(),
            expected: expected.talent.skill.to_string(),
            actual: actual.talent.skill.to_string(),
        });
    }
    if actual.talent.burst != expected.talent.burst {
        diffs.push(FieldDiff {
            field: "talent.burst".into(),
            expected: expected.talent.burst.to_string(),
            actual: actual.talent.burst.to_string(),
        });
    }

    diffs
}

/// Compare weapons using two-phase matching.
///
/// Phase 1: Within each key group, find exact matches (0 diffs) first.
/// Phase 2: Collect all unmatched items as missing/extra, then pair them
/// across all groups by fewest diffs to show useful diff info.
fn diff_weapons(
    actual: &[GoodWeapon],
    expected: &[GoodWeapon],
    summary: &mut DiffSummary,
) -> Vec<WeaponDiff> {
    let mut diffs = Vec::new();

    // Group by key
    let mut expected_by_key: HashMap<&str, Vec<(usize, &GoodWeapon)>> = HashMap::new();
    for (i, w) in expected.iter().enumerate() {
        expected_by_key.entry(w.key.as_str()).or_default().push((i, w));
    }
    let mut actual_by_key: HashMap<&str, Vec<(usize, &GoodWeapon)>> = HashMap::new();
    for (i, w) in actual.iter().enumerate() {
        actual_by_key.entry(w.key.as_str()).or_default().push((i, w));
    }

    let all_keys: std::collections::BTreeSet<&str> = expected_by_key
        .keys()
        .chain(actual_by_key.keys())
        .copied()
        .collect();

    // Phase 1: exact matches within each key group
    let mut unmatched_exp: Vec<(usize, &GoodWeapon)> = Vec::new();
    let mut unmatched_act: Vec<(usize, &GoodWeapon)> = Vec::new();

    for key in all_keys {
        let exp_list = expected_by_key.remove(key).unwrap_or_default();
        let mut act_list = actual_by_key.remove(key).unwrap_or_default();

        for (exp_idx, exp) in &exp_list {
            // Look for exact match only
            let exact = act_list
                .iter()
                .enumerate()
                .find(|(_, (_, act))| diff_weapon_fields(act, exp).is_empty());

            if let Some((pos, _)) = exact {
                act_list.remove(pos);
                summary.weapons_matched += 1;
            } else {
                unmatched_exp.push((*exp_idx, *exp));
            }
        }
        unmatched_act.extend(act_list);
    }

    // Phase 2: pair unmatched expected↔actual by fewest diffs
    for (exp_idx, exp) in &unmatched_exp {
        let best = unmatched_act
            .iter()
            .enumerate()
            .map(|(pos, (_, act))| {
                let fd = diff_weapon_fields(act, exp);
                (pos, fd)
            })
            .min_by_key(|(_, fd)| fd.len());

        if let Some((pos, field_diffs)) = best {
            let (_, act) = unmatched_act.remove(pos);
            summary.weapons_mismatched += 1;
            diffs.push(WeaponDiff {
                index: *exp_idx,
                key_expected: exp.key.clone(),
                key_actual: act.key.clone(),
                status: DiffStatus::Compared,
                field_diffs,
            });
        } else {
            summary.weapons_missing += 1;
            diffs.push(WeaponDiff {
                index: *exp_idx,
                key_expected: exp.key.clone(),
                key_actual: String::new(),
                status: DiffStatus::Missing,
                field_diffs: Vec::new(),
            });
        }
    }

    // Remaining unmatched actual weapons
    for (act_idx, act) in unmatched_act {
        summary.weapons_extra += 1;
        diffs.push(WeaponDiff {
            index: act_idx,
            key_expected: String::new(),
            key_actual: act.key.clone(),
            status: DiffStatus::Extra,
            field_diffs: Vec::new(),
        });
    }

    diffs
}

fn diff_weapon_fields(actual: &GoodWeapon, expected: &GoodWeapon) -> Vec<FieldDiff> {
    let mut diffs = Vec::new();

    if actual.key != expected.key {
        diffs.push(FieldDiff {
            field: "key".into(),
            expected: expected.key.clone(),
            actual: actual.key.clone(),
        });
    }
    if actual.level != expected.level {
        diffs.push(FieldDiff {
            field: "level".into(),
            expected: expected.level.to_string(),
            actual: actual.level.to_string(),
        });
    }
    if actual.ascension != expected.ascension {
        diffs.push(FieldDiff {
            field: "ascension".into(),
            expected: expected.ascension.to_string(),
            actual: actual.ascension.to_string(),
        });
    }
    if actual.refinement != expected.refinement {
        diffs.push(FieldDiff {
            field: "refinement".into(),
            expected: expected.refinement.to_string(),
            actual: actual.refinement.to_string(),
        });
    }
    // Skip rarity comparison when expected is 0 (not provided in groundtruth)
    if expected.rarity != 0 && actual.rarity != expected.rarity {
        diffs.push(FieldDiff {
            field: "rarity".into(),
            expected: expected.rarity.to_string(),
            actual: actual.rarity.to_string(),
        });
    }
    if actual.location != expected.location {
        diffs.push(FieldDiff {
            field: "location".into(),
            expected: expected.location.clone(),
            actual: actual.location.clone(),
        });
    }
    if actual.lock != expected.lock {
        diffs.push(FieldDiff {
            field: "lock".into(),
            expected: expected.lock.to_string(),
            actual: actual.lock.to_string(),
        });
    }

    diffs
}

/// Compare artifacts using two-phase matching.
///
/// Phase 1: Find exact matches (0 diffs) within (setKey, slotKey) groups.
/// Phase 2: Collect all unmatched items, then pair missing↔extra across all
/// groups by fewest diffs (weighted: level/mainStatKey mismatches cost more).
/// This avoids misassigning copies that differ only in lock or substats.
fn diff_artifacts(
    actual: &[GoodArtifact],
    expected: &[GoodArtifact],
    summary: &mut DiffSummary,
) -> Vec<ArtifactDiff> {
    let mut diffs = Vec::new();

    type ArtKey = (String, String);
    fn art_group_key(a: &GoodArtifact) -> ArtKey {
        (a.set_key.clone(), a.slot_key.clone())
    }

    let mut expected_by_key: HashMap<ArtKey, Vec<(usize, &GoodArtifact)>> = HashMap::new();
    for (i, a) in expected.iter().enumerate() {
        expected_by_key.entry(art_group_key(a)).or_default().push((i, a));
    }
    let mut actual_by_key: HashMap<ArtKey, Vec<(usize, &GoodArtifact)>> = HashMap::new();
    for (i, a) in actual.iter().enumerate() {
        actual_by_key.entry(art_group_key(a)).or_default().push((i, a));
    }

    let all_keys: std::collections::BTreeSet<ArtKey> = expected_by_key
        .keys()
        .chain(actual_by_key.keys())
        .cloned()
        .collect();

    // Phase 1: exact matches within each group
    let mut unmatched_exp: Vec<(usize, &GoodArtifact)> = Vec::new();
    let mut unmatched_act: Vec<(usize, &GoodArtifact)> = Vec::new();

    for key in all_keys {
        let exp_list = expected_by_key.remove(&key).unwrap_or_default();
        let mut act_list = actual_by_key.remove(&key).unwrap_or_default();

        for (exp_idx, exp) in &exp_list {
            let exact = act_list
                .iter()
                .enumerate()
                .find(|(_, (_, act))| diff_artifact_fields(act, exp).is_empty());

            if let Some((pos, _)) = exact {
                act_list.remove(pos);
                summary.artifacts_matched += 1;
            } else {
                unmatched_exp.push((*exp_idx, *exp));
            }
        }
        unmatched_act.extend(act_list);
    }

    // Phase 2: pair unmatched expected↔actual by weighted score
    fn artifact_match_score(fd: &[FieldDiff]) -> usize {
        fd.iter().map(|f| {
            match f.field.as_str() {
                "setKey" | "slotKey" | "mainStatKey" => 100,
                "level" | "rarity" => 10,
                _ => 1,
            }
        }).sum()
    }

    for (exp_idx, exp) in &unmatched_exp {
        let best = unmatched_act
            .iter()
            .enumerate()
            .map(|(pos, (_, act))| {
                let fd = diff_artifact_fields(act, exp);
                let score = artifact_match_score(&fd);
                (pos, fd, score)
            })
            .min_by_key(|(_, _, score)| *score);

        if let Some((pos, field_diffs, _)) = best {
            let (_, act) = unmatched_act.remove(pos);
            summary.artifacts_mismatched += 1;
            diffs.push(ArtifactDiff {
                index: *exp_idx,
                set_expected: exp.set_key.clone(),
                set_actual: act.set_key.clone(),
                status: DiffStatus::Compared,
                field_diffs,
            });
        } else {
            summary.artifacts_missing += 1;
            diffs.push(ArtifactDiff {
                index: *exp_idx,
                set_expected: exp.set_key.clone(),
                set_actual: String::new(),
                status: DiffStatus::Missing,
                field_diffs: Vec::new(),
            });
        }
    }

    for (act_idx, act) in unmatched_act {
        summary.artifacts_extra += 1;
        diffs.push(ArtifactDiff {
            index: act_idx,
            set_expected: String::new(),
            set_actual: act.set_key.clone(),
            status: DiffStatus::Extra,
            field_diffs: Vec::new(),
        });
    }

    diffs
}

fn diff_artifact_fields(actual: &GoodArtifact, expected: &GoodArtifact) -> Vec<FieldDiff> {
    let mut diffs = Vec::new();

    if actual.set_key != expected.set_key {
        diffs.push(FieldDiff {
            field: "setKey".into(),
            expected: expected.set_key.clone(),
            actual: actual.set_key.clone(),
        });
    }
    if actual.slot_key != expected.slot_key {
        diffs.push(FieldDiff {
            field: "slotKey".into(),
            expected: expected.slot_key.clone(),
            actual: actual.slot_key.clone(),
        });
    }
    if actual.level != expected.level {
        diffs.push(FieldDiff {
            field: "level".into(),
            expected: expected.level.to_string(),
            actual: actual.level.to_string(),
        });
    }
    // Skip rarity comparison when expected is 0 (not provided in groundtruth)
    if expected.rarity != 0 && actual.rarity != expected.rarity {
        diffs.push(FieldDiff {
            field: "rarity".into(),
            expected: expected.rarity.to_string(),
            actual: actual.rarity.to_string(),
        });
    }
    if actual.main_stat_key != expected.main_stat_key {
        diffs.push(FieldDiff {
            field: "mainStatKey".into(),
            expected: expected.main_stat_key.clone(),
            actual: actual.main_stat_key.clone(),
        });
    }
    if actual.location != expected.location {
        diffs.push(FieldDiff {
            field: "location".into(),
            expected: expected.location.clone(),
            actual: actual.location.clone(),
        });
    }
    if actual.lock != expected.lock {
        diffs.push(FieldDiff {
            field: "lock".into(),
            expected: expected.lock.to_string(),
            actual: actual.lock.to_string(),
        });
    }

    // Compare substats (order-independent)
    diff_substats(&actual.substats, &expected.substats, "substats", &mut diffs);

    // Compare unactivated substats (keys only — values can't be OCR'd from game)
    if !expected.unactivated_substats.is_empty() || !actual.unactivated_substats.is_empty() {
        diff_substats_keys_only(
            &actual.unactivated_substats,
            &expected.unactivated_substats,
            "unactivatedSubstats",
            &mut diffs,
        );
    }

    diffs
}

/// Compare substat lists order-independently.
fn diff_substats(
    actual: &[GoodSubStat],
    expected: &[GoodSubStat],
    prefix: &str,
    diffs: &mut Vec<FieldDiff>,
) {
    if actual.len() != expected.len() {
        diffs.push(FieldDiff {
            field: format!("{}.count", prefix),
            expected: expected.len().to_string(),
            actual: actual.len().to_string(),
        });
        // Don't return early — still compare individual substats for diagnostics
    }

    // Build maps by key for order-independent comparison
    let act_map: HashMap<&str, f64> = actual.iter().map(|s| (s.key.as_str(), s.value)).collect();
    let exp_map: HashMap<&str, f64> = expected.iter().map(|s| (s.key.as_str(), s.value)).collect();

    for exp in expected {
        if let Some(&act_val) = act_map.get(exp.key.as_str()) {
            if (act_val - exp.value).abs() > 0.15 {
                diffs.push(FieldDiff {
                    field: format!("{}.{}", prefix, exp.key),
                    expected: format!("{}", exp.value),
                    actual: format!("{}", act_val),
                });
            }
        } else {
            diffs.push(FieldDiff {
                field: format!("{}.{}", prefix, exp.key),
                expected: format!("{}", exp.value),
                actual: "(missing)".into(),
            });
        }
    }

    for act in actual {
        if !exp_map.contains_key(act.key.as_str()) {
            diffs.push(FieldDiff {
                field: format!("{}.{}", prefix, act.key),
                expected: "(missing)".into(),
                actual: format!("{}", act.value),
            });
        }
    }
}

/// Compare unactivated substat lists by key only (ignore values).
/// Unactivated substat values can't be OCR'd from the game (they show "待激活"),
/// so we only check whether the correct keys are present.
fn diff_substats_keys_only(
    actual: &[GoodSubStat],
    expected: &[GoodSubStat],
    prefix: &str,
    diffs: &mut Vec<FieldDiff>,
) {
    if actual.len() != expected.len() {
        diffs.push(FieldDiff {
            field: format!("{}.count", prefix),
            expected: expected.len().to_string(),
            actual: actual.len().to_string(),
        });
    }

    let act_keys: std::collections::HashSet<&str> = actual.iter().map(|s| s.key.as_str()).collect();
    let exp_keys: std::collections::HashSet<&str> = expected.iter().map(|s| s.key.as_str()).collect();

    for exp in expected {
        if !act_keys.contains(exp.key.as_str()) {
            diffs.push(FieldDiff {
                field: format!("{}.{}", prefix, exp.key),
                expected: "present".into(),
                actual: "(missing)".into(),
            });
        }
    }

    for act in actual {
        if !exp_keys.contains(act.key.as_str()) {
            diffs.push(FieldDiff {
                field: format!("{}.{}", prefix, act.key),
                expected: "(missing)".into(),
                actual: "present".into(),
            });
        }
    }
}

/// Print diff results to stdout in a human-readable format.
pub fn print_diff(result: &DiffResult) {
    let s = &result.summary;

    println!("\n=== GOOD Export Diff ===\n");

    // Characters
    if !result.character_diffs.is_empty() {
        println!("--- Characters ---");
        for d in &result.character_diffs {
            match d.status {
                DiffStatus::Missing => println!("  MISSING: {}", d.key),
                DiffStatus::Extra => println!("  EXTRA:   {}", d.key),
                DiffStatus::Compared => {
                    println!("  DIFF:    {}", d.key);
                    for f in &d.field_diffs {
                        println!("    {}", f);
                    }
                }
            }
        }
        println!();
    }

    // Weapons
    if !result.weapon_diffs.is_empty() {
        println!("--- Weapons ---");
        for d in &result.weapon_diffs {
            match d.status {
                DiffStatus::Missing => println!("  MISSING [{}]: {}", d.index, d.key_expected),
                DiffStatus::Extra => println!("  EXTRA   [{}]: {}", d.index, d.key_actual),
                DiffStatus::Compared => {
                    let label = if d.key_expected == d.key_actual {
                        d.key_actual.clone()
                    } else {
                        format!("{} vs {}", d.key_expected, d.key_actual)
                    };
                    println!("  DIFF    [{}]: {}", d.index, label);
                    for f in &d.field_diffs {
                        println!("    {}", f);
                    }
                }
            }
        }
        println!();
    }

    // Artifacts
    if !result.artifact_diffs.is_empty() {
        println!("--- Artifacts ---");
        for d in &result.artifact_diffs {
            match d.status {
                DiffStatus::Missing => println!("  MISSING [{}]: {}", d.index, d.set_expected),
                DiffStatus::Extra => println!("  EXTRA   [{}]: {}", d.index, d.set_actual),
                DiffStatus::Compared => {
                    let label = if d.set_expected == d.set_actual {
                        d.set_actual.clone()
                    } else {
                        format!("{} vs {}", d.set_expected, d.set_actual)
                    };
                    println!("  DIFF    [{}]: {}", d.index, label);
                    for f in &d.field_diffs {
                        println!("    {}", f);
                    }
                }
            }
        }
        println!();
    }

    // Per-field summary (grouped counts)
    let mut field_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for d in &result.character_diffs {
        for f in &d.field_diffs {
            *field_counts.entry(f.field.clone()).or_default() += 1;
        }
    }
    for d in &result.weapon_diffs {
        for f in &d.field_diffs {
            *field_counts.entry(f.field.clone()).or_default() += 1;
        }
    }
    for d in &result.artifact_diffs {
        for f in &d.field_diffs {
            *field_counts.entry(f.field.clone()).or_default() += 1;
        }
    }

    if !field_counts.is_empty() {
        println!("--- Per-field mismatch counts ---");
        let mut sorted: Vec<_> = field_counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        for (field, count) in &sorted {
            println!("  {:30} {}", field, count);
        }
        println!();
    }

    // Summary
    println!("=== Summary ===");
    println!(
        "Characters: {} matched, {} mismatched, {} missing, {} extra",
        s.characters_matched, s.characters_mismatched, s.characters_missing, s.characters_extra
    );
    println!(
        "Weapons:    {} matched, {} mismatched, {} missing, {} extra",
        s.weapons_matched, s.weapons_mismatched, s.weapons_missing, s.weapons_extra
    );
    println!(
        "Artifacts:  {} matched, {} mismatched, {} missing, {} extra",
        s.artifacts_matched, s.artifacts_mismatched, s.artifacts_missing, s.artifacts_extra
    );
    println!(
        "Total errors: {}",
        s.total_errors()
    );
}
