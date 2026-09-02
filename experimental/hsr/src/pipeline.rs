use crate::{
    error::{hints, HsrError, HsrResult},
    model::{
        ExportGear, ExportPrivacy, ExportReference, ExportSource, ExportStat, GearCategory,
        HsrCharacter, HsrInventoryExport, HsrLightCone, HsrPlanarOrnament, HsrRelic,
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
        reference: ExportReference {
            schema_version: references.schema_version(),
            provider: references.provider().to_string(),
            revision: references.revision().to_string(),
        },
        privacy: ExportPrivacy {
            account_identifiers_included: false,
            raw_packet_data_included: false,
            server_item_identifiers_included: false,
        },
        characters,
        light_cones,
        relics,
        planar_ornaments,
    })
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
