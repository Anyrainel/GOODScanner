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
    if snapshot.evidence.kind != EvidenceKind::SanitizedFixture {
        return observation_error("only sanitizedFixture evidence is enabled".to_string());
    }
    if snapshot.evidence.fixture_version == 0 {
        return observation_error("evidence.fixtureVersion must not be zero".to_string());
    }

    for (index, character) in snapshot.characters.iter().enumerate() {
        if character.character_id == 0 {
            return observation_error(format!("characters[{index}].characterId must not be zero"));
        }
    }
    for (index, light_cone) in snapshot.light_cones.iter().enumerate() {
        if light_cone.light_cone_id == 0 {
            return observation_error(format!("lightCones[{index}].lightConeId must not be zero"));
        }
    }

    for (index, gear) in snapshot.gear.iter().enumerate() {
        if gear.piece_id == 0 || gear.main_stat_id == 0 {
            return observation_error(format!(
                "gear[{index}] pieceId and mainStatId must not be zero"
            ));
        }
        if !gear.main_stat_value.is_finite() {
            return observation_error(format!(
                "gear[{index}] contains a non-finite main-stat value"
            ));
        }
        if !gear.substats.iter().all(|stat| stat.value.is_finite()) {
            return observation_error(format!("gear[{index}] contains a non-finite substat value"));
        }
        let mut stat_ids = std::collections::BTreeSet::new();
        if !gear
            .substats
            .iter()
            .all(|stat| stat_ids.insert(stat.stat_id))
        {
            return observation_error(format!("gear[{index}] contains a duplicate substat statId"));
        }
    }
    Ok(())
}

fn observation_error(detail: String) -> HsrResult<()> {
    Err(HsrError::new(
        "HSR-OBS-INVALID",
        hints::OBSERVATION_INVALID,
        detail,
    ))
}
