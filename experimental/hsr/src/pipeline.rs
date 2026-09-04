use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use crate::{
    error::{hints, HsrError, HsrResult},
    model::{
        AchievementCoverage, AchievementSource, AchievementSourceKind, AchievementStatus,
        CoverageLevel, EvidenceKind, ExportGear, ExportPrivacy, ExportReference, ExportSource,
        ExportStat, GearCategory, HsrAchievementEntry, HsrAchievementSnapshot, HsrCharacter,
        HsrInventoryExport, HsrLightCone, HsrPlanarOrnament, HsrRelic, InventoryCoverage,
        ObservationSnapshot, ObservedGear, EXPORT_SCHEMA, EXPORT_SCHEMA_VERSION,
    },
    observation::ValidatedObservationSnapshot,
    reference::ReferenceCache,
};

/// Convert a sanitized semantic observation into a deterministic experimental
/// export. This function has no live-device or mutation side effects.
pub fn build_export(
    observations: ValidatedObservationSnapshot,
    references: &ReferenceCache,
) -> HsrResult<HsrInventoryExport> {
    build_inventory_export(observations, references)
}

/// Normalize completed public achievement IDs into the strict v3 snapshot
/// contract. Duplicate IDs are collapsed and the output is ascending.
pub fn build_achievement_snapshot<I, S>(
    completed_ids: I,
    revision: S,
    references: &ReferenceCache,
) -> HsrResult<HsrAchievementSnapshot>
where
    I: IntoIterator<Item = u32>,
    S: Into<String>,
{
    require_achievement_references(references)?;
    let revision = revision.into();
    validate_safe_revision(&revision)?;

    let mut normalized = BTreeSet::new();
    for achievement_id in completed_ids {
        if achievement_id == 0 {
            return achievement_error("achievementId must not be zero");
        }
        if !references.has_achievement(achievement_id) {
            return achievement_error("achievementId is absent from the GIlore reference");
        }
        normalized.insert(achievement_id);
    }

    Ok(HsrAchievementSnapshot {
        source: AchievementSource {
            kind: AchievementSourceKind::PacketCapture,
            revision,
        },
        coverage: AchievementCoverage::Complete,
        entries: normalized
            .into_iter()
            .map(|achievement_id| HsrAchievementEntry {
                achievement_id,
                status: AchievementStatus::Completed,
            })
            .collect(),
    })
}

/// Build a mixed export whose inventory and achievement provenance remain
/// independent. The supplied achievement snapshot is revalidated before it is
/// attached so callers cannot serialize a hand-constructed malformed value.
pub fn build_export_with_achievements(
    observations: ValidatedObservationSnapshot,
    achievements: HsrAchievementSnapshot,
    references: &ReferenceCache,
) -> HsrResult<HsrInventoryExport> {
    validate_achievement_snapshot(&achievements, references)?;
    let mut export = build_inventory_export(observations, references)?;
    export.achievements = Some(achievements);
    Ok(export)
}

/// Build an achievement-only patch export. Empty inventory arrays paired with
/// unknown inventory coverage must be merged conservatively by consumers; the
/// nested achievement snapshot remains a complete authoritative replacement.
pub fn build_achievement_only_export(
    achievements: HsrAchievementSnapshot,
    references: &ReferenceCache,
) -> HsrResult<HsrInventoryExport> {
    validate_achievement_snapshot(&achievements, references)?;
    Ok(HsrInventoryExport {
        schema: EXPORT_SCHEMA.to_string(),
        schema_version: EXPORT_SCHEMA_VERSION,
        source: ExportSource {
            kind: EvidenceKind::PacketCapture,
            revision: achievements.source.revision.clone(),
            coverage: InventoryCoverage {
                characters: CoverageLevel::Unknown,
                light_cones: CoverageLevel::Unknown,
                relics: CoverageLevel::Unknown,
            },
        },
        reference: export_reference(references),
        privacy: privacy_contract(),
        characters: Vec::new(),
        light_cones: Vec::new(),
        relics: Vec::new(),
        planar_ornaments: Vec::new(),
        achievements: Some(achievements),
    })
}

/// Serialize a v3 export without overwriting an existing path. A failed write
/// makes a best effort to remove the newly-created partial file.
pub fn write_export_create_new(path: &Path, export: &HsrInventoryExport) -> HsrResult<()> {
    let output = serde_json::to_vec_pretty(export).map_err(|error| {
        HsrError::write_failed(
            "HSR-EXPORT-JSON",
            format!("serialization failed; cause={error}"),
        )
    })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            HsrError::write_failed(
                "HSR-EXPORT-CREATE",
                format!("path={}; cause={error}", path.display()),
            )
        })?;
    let write_result = file
        .write_all(&output)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = write_result {
        let cleanup = fs::remove_file(path)
            .err()
            .map(|cleanup| format!("; partial-file cleanup failed; cause={cleanup}"))
            .unwrap_or_default();
        return Err(HsrError::write_failed(
            "HSR-EXPORT-WRITE",
            format!("path={}; cause={error}{cleanup}", path.display()),
        ));
    }
    Ok(())
}

fn build_inventory_export(
    observations: ValidatedObservationSnapshot,
    references: &ReferenceCache,
) -> HsrResult<HsrInventoryExport> {
    let ObservationSnapshot {
        evidence,
        characters: observed_characters,
        light_cones: observed_light_cones,
        gear: observed_gear,
        ..
    } = observations.into_inner();

    let characters = observed_characters
        .into_iter()
        .enumerate()
        .map(|(index, observed)| {
            let context = format!("characters[{index}]");
            let reference = references
                .character(observed.character_id)
                .ok_or_else(|| missing_reference("character", &context))?;
            Ok(HsrCharacter {
                local_id: synthetic_local_id("character", index),
                key: reference.key.clone(),
                game_id: reference.game_id,
                name: reference.name.clone(),
                rarity: reference.rarity,
                path: reference.path.clone(),
                level: observed.level,
                ascension: observed.ascension,
                eidolon: observed.eidolon,
            })
        })
        .collect::<HsrResult<Vec<_>>>()?;

    let light_cones = observed_light_cones
        .into_iter()
        .enumerate()
        .map(|(index, observed)| {
            let context = format!("lightCones[{index}]");
            let reference = references
                .light_cone(observed.light_cone_id)
                .ok_or_else(|| missing_reference("lightCone", &context))?;
            Ok(HsrLightCone {
                local_id: synthetic_local_id("light-cone", index),
                key: reference.key.clone(),
                game_id: reference.game_id,
                name: reference.name.clone(),
                rarity: reference.rarity,
                path: reference.path.clone(),
                level: observed.level,
                ascension: observed.ascension,
                superimposition: observed.superimposition,
                location_key: resolve_location(
                    observed.equipped_character_id,
                    references,
                    &context,
                )?,
                lock: observed.lock,
            })
        })
        .collect::<HsrResult<Vec<_>>>()?;

    let mut relics = Vec::new();
    let mut planar_ornaments = Vec::new();
    for (index, observed) in observed_gear.into_iter().enumerate() {
        let context = format!("gear[{index}]");
        let reference = references
            .canonical_gear_for_observation(
                observed.piece_id,
                &observed.main_stat_key,
                observed.level,
                observed.main_stat_value,
            )
            .ok_or_else(|| missing_reference("gearVisibleIdentity", &context))?;
        let category = reference.category;
        match category {
            GearCategory::Relic => {
                let local_id = synthetic_local_id("relic", relics.len());
                let gear = resolve_gear(observed, local_id, reference, references, &context)?;
                relics.push(HsrRelic { gear });
            },
            GearCategory::PlanarOrnament => {
                let local_id = synthetic_local_id("planar", planar_ornaments.len());
                let gear = resolve_gear(observed, local_id, reference, references, &context)?;
                planar_ornaments.push(HsrPlanarOrnament { gear });
            },
        }
    }

    Ok(HsrInventoryExport {
        schema: EXPORT_SCHEMA.to_string(),
        schema_version: EXPORT_SCHEMA_VERSION,
        source: ExportSource {
            kind: evidence.kind,
            revision: evidence.revision,
            coverage: evidence.coverage,
        },
        reference: export_reference(references),
        privacy: privacy_contract(),
        characters,
        light_cones,
        relics,
        planar_ornaments,
        achievements: None,
    })
}

fn export_reference(references: &ReferenceCache) -> ExportReference {
    ExportReference {
        schema_version: references.schema_version(),
        provider: references.provider().to_string(),
        revision: references.revision().to_string(),
    }
}

fn privacy_contract() -> ExportPrivacy {
    ExportPrivacy {
        account_identifiers_included: false,
        raw_packet_data_included: false,
        server_item_identifiers_included: false,
    }
}

fn validate_achievement_snapshot(
    snapshot: &HsrAchievementSnapshot,
    references: &ReferenceCache,
) -> HsrResult<()> {
    require_achievement_references(references)?;
    validate_safe_revision(&snapshot.source.revision)?;
    let mut previous = None;
    for entry in &snapshot.entries {
        if entry.achievement_id == 0 {
            return achievement_error("achievementId must not be zero");
        }
        if previous.is_some_and(|value| value >= entry.achievement_id) {
            return achievement_error("achievement entries must be strictly ascending and unique");
        }
        if !references.has_achievement(entry.achievement_id) {
            return achievement_error("achievementId is absent from the GIlore reference");
        }
        previous = Some(entry.achievement_id);
    }
    Ok(())
}

fn require_achievement_references(references: &ReferenceCache) -> HsrResult<()> {
    if references.achievement_count() == 0 {
        achievement_error("GIlore reference has no achievement IDs")
    } else {
        Ok(())
    }
}

fn validate_safe_revision(revision: &str) -> HsrResult<()> {
    if revision.is_empty()
        || revision.len() > 128
        || !revision
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
    {
        achievement_error("achievement source.revision must be a safe 1-128 character identifier")
    } else {
        Ok(())
    }
}

fn achievement_error<T>(detail: impl Into<String>) -> HsrResult<T> {
    Err(HsrError::new(
        "HSR-ACHIEVEMENT-INVALID",
        hints::OBSERVATION_INVALID,
        detail,
    ))
}

fn resolve_gear(
    observed: ObservedGear,
    local_id: String,
    reference: &crate::model::GearReference,
    references: &ReferenceCache,
    context: &str,
) -> HsrResult<ExportGear> {
    let main_reference = references
        .stat(&observed.main_stat_key)
        .ok_or_else(|| missing_reference("mainStat", context))?;
    let main_stat_value = references
        .relic_main_stat_value_for_piece(reference, &observed.main_stat_key, observed.level)
        .ok_or_else(|| missing_reference("mainStatProgression", context))?;

    let mut substats = observed
        .substats
        .into_iter()
        .map(|observed_stat| {
            let stat_reference = references
                .stat(&observed_stat.stat_key)
                .ok_or_else(|| missing_reference("substat", context))?;
            Ok(ExportStat {
                key: stat_reference.key.clone(),
                name: stat_reference.name.clone(),
                value: observed_stat.value,
            })
        })
        .collect::<HsrResult<Vec<_>>>()?;
    substats.sort_by(|left, right| left.key.cmp(&right.key));

    Ok(ExportGear {
        local_id,
        key: reference.key.clone(),
        game_id: reference.game_id,
        name: reference.name.clone(),
        set_key: reference.set_key.clone(),
        set_name: reference.set_name.clone(),
        rarity: reference.rarity,
        slot: reference.slot,
        level: observed.level,
        main_stat: ExportStat {
            key: main_reference.key.clone(),
            name: main_reference.name.clone(),
            value: main_stat_value,
        },
        substats,
        location_key: resolve_location(observed.equipped_character_id, references, context)?,
        lock: observed.lock,
        discard: observed.discard,
    })
}

fn resolve_location(
    equipped_character_id: Option<u32>,
    references: &ReferenceCache,
    context: &str,
) -> HsrResult<Option<String>> {
    equipped_character_id
        .map(|game_id| {
            references
                .character(game_id)
                .map(|reference| reference.key.clone())
                .ok_or_else(|| missing_reference("equippedCharacter", context))
        })
        .transpose()
}

fn synthetic_local_id(prefix: &str, index: usize) -> String {
    format!("{prefix}-{:03}", index + 1)
}

fn missing_reference(kind: &str, context: &str) -> HsrError {
    HsrError::new(
        "HSR-REF-MISSING",
        hints::REFERENCE_MISSING,
        format!("kind={kind}; record={context}; observed identifier redacted"),
    )
}
