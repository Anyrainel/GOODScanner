use std::{fs, path::PathBuf};

use serde_json::Value;

use crate::{
    error::{hints, HsrError, HsrResult},
    model::{EvidenceKind, ObservationSnapshot, OBSERVATION_SCHEMA_VERSION},
    privacy::reject_sensitive_fields,
};

pub trait ObservationSource {
    fn load(&self) -> HsrResult<ValidatedObservationSnapshot>;
}

/// Observation data that has passed the sensitive-field scan, strict typed
/// deserialization, and semantic validation. Its inner value is intentionally
/// private so callers cannot bypass those gates before export.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedObservationSnapshot(ObservationSnapshot);

impl ValidatedObservationSnapshot {
    pub(crate) fn into_inner(self) -> ObservationSnapshot {
        self.0
    }

    pub(crate) fn as_inner(&self) -> &ObservationSnapshot {
        &self.0
    }

    /// Construct a scanner-produced snapshot through the same semantic gates
    /// used for fixture input. Callers cannot construct a validated value
    /// without passing this check.
    pub fn from_screen_capture(snapshot: ObservationSnapshot) -> HsrResult<Self> {
        if snapshot.evidence.kind != EvidenceKind::ScreenCapture {
            return observation_error(
                "screen capture constructor requires screenCapture evidence".to_string(),
            );
        }
        validate_snapshot(&snapshot)?;
        Ok(Self(snapshot))
    }

    /// Construct a packet-capture snapshot only after the capture adapter has
    /// discarded server/account identifiers and normalized public semantics.
    pub fn from_packet_capture(snapshot: ObservationSnapshot) -> HsrResult<Self> {
        if snapshot.evidence.kind != EvidenceKind::PacketCapture {
            return observation_error(
                "packet capture constructor requires packetCapture evidence".to_string(),
            );
        }
        validate_snapshot(&snapshot)?;
        Ok(Self(snapshot))
    }
}

/// File source restricted to sanitized semantic fixtures. It deliberately does
/// not accept raw packet payloads, capture credentials, or account metadata.
#[derive(Debug, Clone)]
pub struct FixtureObservationSource {
    path: PathBuf,
}

impl FixtureObservationSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl ObservationSource for FixtureObservationSource {
    fn load(&self) -> HsrResult<ValidatedObservationSnapshot> {
        let text = fs::read_to_string(&self.path).map_err(|error| {
            HsrError::new(
                "HSR-OBS-READ",
                hints::READ_FAILED,
                format!("path={}; cause={error}", self.path.display()),
            )
        })?;
        parse_sanitized_fixture(&text).map_err(|error| {
            HsrError::new(
                error.code(),
                match error.code() {
                    "HSR-DATA-SENSITIVE" => hints::SENSITIVE_DATA,
                    "HSR-OBS-JSON" => hints::JSON_INVALID,
                    _ => hints::OBSERVATION_INVALID,
                },
                format!("path={}; cause={error}", self.path.display()),
            )
        })
    }
}

pub fn parse_sanitized_fixture(text: &str) -> HsrResult<ValidatedObservationSnapshot> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| HsrError::new("HSR-OBS-JSON", hints::JSON_INVALID, error.to_string()))?;
    reject_sensitive_fields(&value)?;

    let snapshot: ObservationSnapshot = serde_json::from_value(value)
        .map_err(|error| HsrError::new("HSR-OBS-JSON", hints::JSON_INVALID, error.to_string()))?;
    if snapshot.evidence.kind != EvidenceKind::SanitizedFixture {
        return observation_error("fixture input requires sanitizedFixture evidence".to_string());
    }
    validate_snapshot(&snapshot)?;
    Ok(ValidatedObservationSnapshot(snapshot))
}

fn validate_snapshot(snapshot: &ObservationSnapshot) -> HsrResult<()> {
    if snapshot.schema_version != OBSERVATION_SCHEMA_VERSION {
        return observation_error(format!(
            "unsupported schemaVersion={}; expected={OBSERVATION_SCHEMA_VERSION}",
            snapshot.schema_version
        ));
    }
    if snapshot.evidence.revision.trim().is_empty()
        || snapshot.evidence.revision.len() > 128
        || !snapshot
            .evidence
            .revision
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
    {
        return observation_error(
            "evidence.revision must be a safe 1-128 character identifier".to_string(),
        );
    }

    let mut character_ids = std::collections::BTreeSet::new();
    for (index, character) in snapshot.characters.iter().enumerate() {
        if character.character_id == 0
            || !character_ids.insert(character.character_id)
            || !(1..=100).contains(&character.level)
            || character.ascension > 8
            || character.eidolon > 6
        {
            return observation_error(format!(
                "characters[{index}] must have a unique nonzero characterId, level 1-100, ascension 0-8, and eidolon 0-6"
            ));
        }
    }
    for (index, light_cone) in snapshot.light_cones.iter().enumerate() {
        if light_cone.light_cone_id == 0
            || !(1..=100).contains(&light_cone.level)
            || light_cone.ascension > 8
            || !(1..=5).contains(&light_cone.superimposition)
            || light_cone.equipped_character_id == Some(0)
        {
            return observation_error(format!(
                "lightCones[{index}] must have a nonzero lightConeId, level 1-100, ascension 0-8, superimposition 1-5, and no zero equippedCharacterId"
            ));
        }
    }

    for (index, gear) in snapshot.gear.iter().enumerate() {
        if gear.piece_id == 0
            || gear.level > 15
            || gear.main_stat_key.trim().is_empty()
            || gear.equipped_character_id == Some(0)
        {
            return observation_error(format!(
                "gear[{index}] must have a nonzero pieceId, level 0-15, nonempty mainStatKey, and no zero equippedCharacterId"
            ));
        }
        if !gear.main_stat_value.is_finite() || gear.main_stat_value < 0.0 {
            return observation_error(format!("gear[{index}] contains an invalid main-stat value"));
        }
        if gear.substats.len() > 4 {
            return observation_error(format!(
                "gear[{index}] contains more than four visible substats"
            ));
        }
        if !gear
            .substats
            .iter()
            .all(|stat| stat.value.is_finite() && stat.value >= 0.0)
        {
            return observation_error(format!("gear[{index}] contains an invalid substat value"));
        }
        let mut stat_keys = std::collections::BTreeSet::new();
        if !gear
            .substats
            .iter()
            .all(|stat| !stat.stat_key.trim().is_empty() && stat_keys.insert(&stat.stat_key))
        {
            return observation_error(format!(
                "gear[{index}] contains an empty or duplicate substat statKey"
            ));
        }
        if gear
            .substats
            .iter()
            .any(|stat| stat.stat_key == gear.main_stat_key)
        {
            return observation_error(format!(
                "gear[{index}] repeats its main stat in visible substats"
            ));
        }
    }
    Ok(())
}

fn observation_error<T>(detail: String) -> HsrResult<T> {
    Err(HsrError::new(
        "HSR-OBS-INVALID",
        hints::OBSERVATION_INVALID,
        detail,
    ))
}
