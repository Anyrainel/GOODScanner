use std::{collections::BTreeMap, fs, path::PathBuf};

use serde_json::Value;

use crate::{
    error::{hints, HsrError, HsrResult},
    model::{
        CharacterReference, GearReference, LightConeReference, ReferenceSnapshot, StatReference,
        REFERENCE_SCHEMA_VERSION,
    },
    privacy::reject_sensitive_fields,
};

/// Boundary for normalized HSR reference data. A future GIlore adapter should
/// produce this snapshot rather than leaking datamine-specific structures into
/// the scanner pipeline.
pub trait ReferenceProvider {
    fn load(&self) -> HsrResult<ReferenceSnapshot>;
}

#[derive(Debug, Clone)]
pub struct JsonFileReferenceProvider {
    path: PathBuf,
}

impl JsonFileReferenceProvider {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl ReferenceProvider for JsonFileReferenceProvider {
    fn load(&self) -> HsrResult<ReferenceSnapshot> {
        let text = fs::read_to_string(&self.path).map_err(|error| {
            HsrError::new(
                "HSR-REF-READ",
                hints::READ_FAILED,
                format!("path={}; cause={error}", self.path.display()),
            )
        })?;
        let value: Value = serde_json::from_str(&text).map_err(|error| {
            HsrError::new(
                "HSR-REF-JSON",
                hints::JSON_INVALID,
                format!("path={}; cause={error}", self.path.display()),
            )
        })?;
        reject_sensitive_fields(&value).map_err(|error| {
            HsrError::new(
                error.code(),
                hints::SENSITIVE_DATA,
                format!("path={}; cause={error}", self.path.display()),
            )
        })?;
        serde_json::from_value(value).map_err(|error| {
            HsrError::new(
                "HSR-REF-JSON",
                hints::JSON_INVALID,
                format!("path={}; cause={error}", self.path.display()),
            )
        })
    }
}

#[derive(Debug, Clone)]
pub struct ReferenceCache {
    schema_version: u32,
    provider: String,
    revision: String,
    characters: BTreeMap<u32, CharacterReference>,
    light_cones: BTreeMap<u32, LightConeReference>,
    gear_pieces: BTreeMap<u32, GearReference>,
    stats: BTreeMap<u32, StatReference>,
}

impl ReferenceCache {
    pub fn from_provider(provider: &dyn ReferenceProvider) -> HsrResult<Self> {
        Self::from_snapshot(provider.load()?)
    }

    pub fn from_snapshot(snapshot: ReferenceSnapshot) -> HsrResult<Self> {
        validate_snapshot_header(&snapshot)?;

        let characters = collect_unique(
            snapshot.characters,
            "characters",
            |reference| reference.game_id,
            validate_character,
        )?;
        let light_cones = collect_unique(
            snapshot.light_cones,
            "lightCones",
            |reference| reference.game_id,
            validate_light_cone,
        )?;
        let gear_pieces = collect_unique(
            snapshot.gear_pieces,
            "gearPieces",
            |reference| reference.game_id,
            validate_gear,
        )?;
        let stats = collect_unique(
            snapshot.stats,
            "stats",
            |reference| reference.game_id,
            validate_stat,
        )?;

        ensure_unique_keys(
            "characters",
            characters.values().map(|reference| reference.key.as_str()),
        )?;
        ensure_unique_keys(
            "lightCones",
            light_cones.values().map(|reference| reference.key.as_str()),
        )?;
        ensure_unique_keys(
            "gearPieces",
            gear_pieces.values().map(|reference| reference.key.as_str()),
        )?;
        ensure_unique_keys(
            "stats",
            stats.values().map(|reference| reference.key.as_str()),
        )?;

        Ok(Self {
            schema_version: snapshot.schema_version,
            provider: snapshot.provider,
            revision: snapshot.revision,
            characters,
            light_cones,
            gear_pieces,
            stats,
        })
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn character(&self, game_id: u32) -> Option<&CharacterReference> {
        self.characters.get(&game_id)
    }

    pub fn light_cone(&self, game_id: u32) -> Option<&LightConeReference> {
        self.light_cones.get(&game_id)
    }

    pub fn gear(&self, game_id: u32) -> Option<&GearReference> {
        self.gear_pieces.get(&game_id)
    }

    pub fn stat(&self, game_id: u32) -> Option<&StatReference> {
        self.stats.get(&game_id)
    }
}

fn validate_snapshot_header(snapshot: &ReferenceSnapshot) -> HsrResult<()> {
    if snapshot.schema_version != REFERENCE_SCHEMA_VERSION {
        return reference_error(format!(
            "unsupported schemaVersion={}; expected={REFERENCE_SCHEMA_VERSION}",
            snapshot.schema_version
        ));
    }
    require_text("provider", &snapshot.provider)?;
    require_text("revision", &snapshot.revision)?;
    require_safe_identifier("provider", &snapshot.provider)?;
    require_safe_identifier("revision", &snapshot.revision)
}

fn validate_character(reference: &CharacterReference) -> HsrResult<()> {
    validate_common_reference(
        "character",
        reference.game_id,
        &reference.key,
        &reference.name,
        reference.rarity,
    )?;
    require_text("character.path", &reference.path)
}

fn validate_light_cone(reference: &LightConeReference) -> HsrResult<()> {
    validate_common_reference(
        "lightCone",
        reference.game_id,
        &reference.key,
        &reference.name,
        reference.rarity,
    )?;
    require_text("lightCone.path", &reference.path)
}

fn validate_gear(reference: &GearReference) -> HsrResult<()> {
    validate_common_reference(
        "gear",
        reference.game_id,
        &reference.key,
        &reference.name,
        reference.rarity,
    )?;
    require_text("gear.setKey", &reference.set_key)?;
    if !reference.set_name.is_complete() {
        return reference_error(format!(
            "gear gameId={} has an incomplete bilingual setName",
            reference.game_id
        ));
    }
    if reference.slot.category() != reference.category {
        return reference_error(format!(
            "gear gameId={} category does not match slot {:?}",
            reference.game_id, reference.slot
        ));
    }
    Ok(())
}

fn validate_stat(reference: &StatReference) -> HsrResult<()> {
    if reference.game_id == 0 {
        return reference_error("stat gameId must not be zero".to_string());
    }
    require_text("stat.key", &reference.key)?;
    if !reference.name.is_complete() {
        return reference_error(format!(
            "stat gameId={} has an incomplete bilingual name",
            reference.game_id
        ));
    }
    Ok(())
}

fn validate_common_reference(
    kind: &str,
    game_id: u32,
    key: &str,
    name: &crate::localization::BilingualName,
    rarity: u8,
) -> HsrResult<()> {
    if game_id == 0 {
        return reference_error(format!("{kind} gameId must not be zero"));
    }
    require_text(&format!("{kind}.key"), key)?;
    if !name.is_complete() {
        return reference_error(format!(
            "{kind} gameId={game_id} has an incomplete bilingual name"
        ));
    }
    if rarity == 0 {
        return reference_error(format!("{kind} gameId={game_id} rarity must not be zero"));
    }
    Ok(())
}

fn collect_unique<T, F, V>(
    values: Vec<T>,
    collection: &str,
    key: F,
    validate: V,
) -> HsrResult<BTreeMap<u32, T>>
where
    F: Fn(&T) -> u32,
    V: Fn(&T) -> HsrResult<()>,
{
    let mut result = BTreeMap::new();
    for value in values {
        validate(&value)?;
        let game_id = key(&value);
        if result.insert(game_id, value).is_some() {
            return reference_error(format!("duplicate gameId={game_id} in {collection}"));
        }
    }
    Ok(result)
}

fn require_text(field: &str, value: &str) -> HsrResult<()> {
    if value.trim().is_empty() {
        reference_error(format!("{field} must not be empty"))
    } else {
        Ok(())
    }
}

fn require_safe_identifier(field: &str, value: &str) -> HsrResult<()> {
    if value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        reference_error(format!(
            "{field} must be a 1-128 character ASCII identifier"
        ))
    } else {
        Ok(())
    }
}

fn ensure_unique_keys<'a>(
    collection: &str,
    keys: impl IntoIterator<Item = &'a str>,
) -> HsrResult<()> {
    let mut seen = std::collections::BTreeSet::new();
    for key in keys {
        if !seen.insert(key) {
            return reference_error(format!("duplicate key={key} in {collection}"));
        }
    }
    Ok(())
}

fn reference_error<T>(detail: String) -> HsrResult<T> {
    Err(HsrError::new(
        "HSR-REF-INVALID",
        hints::REFERENCE_INVALID,
        detail,
    ))
}
