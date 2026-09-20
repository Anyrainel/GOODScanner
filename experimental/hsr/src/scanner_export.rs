//! HSR-Scanner v4 interchange, as consumed by Fribbels and Reliquary clients.
//! `source` is the importer's required format discriminator; `generator` records
//! the actual producer. Account and server-instance identifiers are not exported.
//! See `docs/HSR_EXPORT.md`.
//!
//! Screenshot OCR and packet capture both write this shape. OCR omits fields it
//! cannot observe rather than emitting a second schema.

use crate::{
    model::*, observation::ValidatedObservationSnapshot, reference::ReferenceCache, HsrError,
    HsrResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CharacterDetails {
    /// Packet skill-tree revision. Screenshot scans omit this; it is not on screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ability_version: Option<u32>,
    pub skills: BTreeMap<String, u32>,
    pub traces: BTreeMap<String, bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memosprite: Option<BTreeMap<String, u32>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CaptureExportDetails {
    pub characters: BTreeMap<u32, CharacterDetails>,
    pub trailblazer: Option<String>,
    pub current_trailblazer_path: Option<String>,
}

pub fn build_scanner_export(
    snapshot: &ObservationSnapshot,
    refs: &ReferenceCache,
    details: &CaptureExportDetails,
    completed_achievement_ids: Option<&[u32]>,
) -> HsrResult<Value> {
    let mut characters = Vec::new();
    for c in &snapshot.characters {
        let r = refs
            .character(c.character_id)
            .ok_or_else(|| invalid("unknown character"))?;
        let mut value = json!({"id": c.character_id.to_string(), "name": r.name.en,
            "path": path_name(&r.path)?, "level": c.level, "ascension": c.ascension, "eidolon": c.eidolon});
        if let Some(detail) = details.characters.get(&c.character_id) {
            value.as_object_mut().unwrap().extend(
                serde_json::to_value(detail)
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            );
        }
        characters.push(value);
    }
    let mut cones = Vec::new();
    for (index, c) in snapshot.light_cones.iter().enumerate() {
        let r = refs
            .light_cone(c.light_cone_id)
            .ok_or_else(|| invalid("unknown Light Cone"))?;
        // TODO(ocr): Chinese 「装备中」 has no character name; location stays "".
        // v4 requires a lock boolean; an unread lock icon serializes as false.
        cones.push(json!({"id": c.light_cone_id.to_string(), "name": r.name.en,
            "level": c.level, "ascension": c.ascension, "superimposition": c.superimposition,
            "location": location(c.equipped_character_id), "lock": c.lock.unwrap_or(false),
            "_uid": (index + 1).to_string()}));
    }
    let mut relics = Vec::new();
    for (index, g) in snapshot.gear.iter().enumerate() {
        let r = refs
            .gear(g.piece_id)
            .ok_or_else(|| invalid("unknown relic"))?;
        r.set_key
            .parse::<u32>()
            .map_err(|_| invalid("relic set ID must be numeric"))?;
        let subs = g
            .substats
            .iter()
            .map(|s| {
                Ok(json!({"key": substat_name(&s.stat_key)?,
            "value": s.value}))
            })
            .collect::<HsrResult<Vec<_>>>()?;
        // TODO(ocr): equipped character is often unknown from the portrait row.
        // v4 requires lock/discard booleans; unread icons serialize as false.
        relics.push(json!({"set_id": r.set_key, "name": r.set_name.en,
            "slot": match r.slot { GearSlot::Head => "Head", GearSlot::Hands => "Hands",
                GearSlot::Body => "Body", GearSlot::Feet => "Feet", GearSlot::PlanarSphere => "Planar Sphere",
                GearSlot::LinkRope => "Link Rope" },
            "rarity": r.rarity, "level": g.level, "mainstat": mainstat_name(&g.main_stat_key)?,
            "substats": subs, "location": location(g.equipped_character_id),
            "lock": g.lock.unwrap_or(false), "discard": g.discard.unwrap_or(false),
            "_uid": (index + 1).to_string()}));
    }
    let trailblazer = details.trailblazer.as_deref().or_else(|| {
        snapshot
            .characters
            .iter()
            .find_map(|c| trailblazer_gender(c.character_id))
    });
    let mut metadata = Map::new();
    metadata.insert("uid".into(), Value::Null);
    metadata.insert("trailblazer".into(), json!(trailblazer));
    if let Some(path) = &details.current_trailblazer_path {
        metadata.insert("current_trailblazer_path".into(), json!(path));
    }
    // TODO(ocr): ability_version is a packet skill-tree revision, not on-screen text.
    let mut export = json!({"source": "HSR-Scanner", "build": "v1.2.0", "version": 4,
        "generator": {"name": "GOODScanner", "version": env!("CARGO_PKG_VERSION"),
            "captureRevision": snapshot.evidence.revision, "compatibility": "HSR-Scanner v1.2.0 / format v4"},
        "metadata": metadata,
        "coverage": snapshot.evidence.coverage, "characters": characters, "light_cones": cones, "relics": relics});
    // Presence is authoritative, including []. Absence means achievements were not observed.
    // TODO(ocr): completed achievements are not visible inventory text; omit the field.
    if let Some(ids) = completed_achievement_ids {
        export
            .as_object_mut()
            .unwrap()
            .insert("achievements".into(), json!(ids));
    }
    Ok(export)
}

/// Same v4 document the capture GUI writes. Screenshot scans fill
/// [`CaptureExportDetails`] from visible traces/skills text when those screens
/// were read; packet-only fields stay omitted.
pub fn export_observations(
    snapshot: &ValidatedObservationSnapshot,
    refs: &ReferenceCache,
    details: &CaptureExportDetails,
    completed_achievement_ids: Option<&[u32]>,
) -> HsrResult<Value> {
    build_scanner_export(
        snapshot.as_inner(),
        refs,
        details,
        completed_achievement_ids,
    )
}

fn location(id: Option<u32>) -> String {
    id.map(|id| id.to_string()).unwrap_or_default()
}

pub(crate) fn trailblazer_gender(avatar_id: u32) -> Option<&'static str> {
    (8001..9000)
        .contains(&avatar_id)
        .then_some(if avatar_id % 2 == 0 {
            "Stelle"
        } else {
            "Caelus"
        })
}

pub(crate) fn path_name(path: &str) -> HsrResult<&str> {
    Ok(match path {
        "Warrior" => "Destruction",
        "Rogue" => "Hunt",
        "Mage" => "Erudition",
        "Shaman" => "Harmony",
        "Warlock" => "Nihility",
        "Knight" => "Preservation",
        "Priest" => "Abundance",
        "Memory" => "Remembrance",
        "Elation" => "Elation",
        "Destruction" | "Hunt" | "Erudition" | "Harmony" | "Nihility" | "Preservation"
        | "Abundance" | "Remembrance" => path,
        _ => return Err(invalid(format!("unknown path {path}"))),
    })
}
fn mainstat_name(key: &str) -> HsrResult<&'static str> {
    Ok(match key {
        "HPDelta" | "HPAddedRatio" => "HP",
        "AttackDelta" | "AttackAddedRatio" => "ATK",
        "DefenceAddedRatio" => "DEF",
        "CriticalChanceBase" => "CRIT Rate",
        "CriticalDamageBase" => "CRIT DMG",
        "HealRatioBase" => "Outgoing Healing Boost",
        "SpeedDelta" => "SPD",
        "StatusProbabilityBase" => "Effect Hit Rate",
        "PhysicalAddedRatio" => "Physical DMG Boost",
        "FireAddedRatio" => "Fire DMG Boost",
        "IceAddedRatio" => "Ice DMG Boost",
        "ThunderAddedRatio" => "Lightning DMG Boost",
        "WindAddedRatio" => "Wind DMG Boost",
        "QuantumAddedRatio" => "Quantum DMG Boost",
        "ImaginaryAddedRatio" => "Imaginary DMG Boost",
        "BreakDamageAddedRatioBase" => "Break Effect",
        "SPRatioBase" => "Energy Regeneration Rate",
        _ => return Err(invalid(format!("unknown main stat {key}"))),
    })
}
fn substat_name(key: &str) -> HsrResult<&'static str> {
    Ok(match key {
        "HPDelta" => "HP",
        "AttackDelta" => "ATK",
        "DefenceDelta" => "DEF",
        "SpeedDelta" => "SPD",
        "HPAddedRatio" => "HP_",
        "AttackAddedRatio" => "ATK_",
        "DefenceAddedRatio" => "DEF_",
        "CriticalChanceBase" => "CRIT Rate_",
        "CriticalDamageBase" => "CRIT DMG_",
        "StatusProbabilityBase" => "Effect Hit Rate_",
        "StatusResistanceBase" => "Effect RES_",
        "BreakDamageAddedRatioBase" => "Break Effect_",
        _ => return Err(invalid(format!("unknown substat {key}"))),
    })
}
fn invalid(detail: impl Into<String>) -> HsrError {
    HsrError::write_failed("HSR-SCANNER-EXPORT", detail)
}
