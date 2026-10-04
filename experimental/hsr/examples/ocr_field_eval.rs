//! Offline per-field OCR model comparison on `live_scan` dumps.
//!
//! Every dumped frame is parsed with the production parser once per
//! (field group, model) pair: the group reads through the candidate model and
//! every other field through PaddleOCR v4. An item counts as correct when the
//! parsed result exists in a packet-capture export of the same account, so the
//! result reflects the real parse/validation path, not raw string equality.
//!
//! ```text
//! cargo run --release -p hsr_scanner --example ocr_field_eval -- <debug_images> <capture.json>
//! ```

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use hsr_scanner::{
    build_scanner_export,
    ocr::{InventoryKind, OcrField, OcrModels, PaddleOcrReader, PanelParser},
    CaptureExportDetails, CoverageLevel, EvidenceKind, InventoryCoverage, ObservationEvidence,
    ObservationSnapshot, ObservedGear, ObservedLightCone, ReferenceCache,
};
use image::RgbImage;
use serde_json::Value;
use yas::ocr::ImageToText;

const MODELS: [&str; 3] = ["ppocrv4", "ppocrv5", "ppocrv6tiny"];
const BASELINE: usize = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Uid,
    CharacterName,
    CharacterLevel,
    CharacterSkill,
    GearName,
    GearLevel,
    GearMainName,
    GearMainValue,
    GearSubName,
    GearSubValue,
    LightConeName,
    LightConeLevel,
    LightConeSuperimposition,
}

impl Group {
    fn of(field: OcrField) -> Option<Self> {
        Some(match field {
            OcrField::Uid => Self::Uid,
            OcrField::CharacterName => Self::CharacterName,
            OcrField::CharacterLevel => Self::CharacterLevel,
            OcrField::CharacterSkill(_) => Self::CharacterSkill,
            OcrField::GearName => Self::GearName,
            OcrField::GearLevel => Self::GearLevel,
            OcrField::GearMainName => Self::GearMainName,
            OcrField::GearMainValue => Self::GearMainValue,
            OcrField::GearSubName(_) => Self::GearSubName,
            OcrField::GearSubValue(_) => Self::GearSubValue,
            OcrField::LightConeName => Self::LightConeName,
            OcrField::LightConeLevel => Self::LightConeLevel,
            OcrField::LightConeSuperimposition => Self::LightConeSuperimposition,
            OcrField::MenuTitle
            | OcrField::InventoryQuantity
            | OcrField::GearEquipped
            | OcrField::LightConeEquipped => return None,
        })
    }
}

/// Routes `group` (or every field when `group` is `None`) to `candidate`.
struct Routed<'a> {
    models: &'a [Box<dyn ImageToText<RgbImage> + Send>],
    group: Option<Group>,
    candidate: usize,
}

impl OcrModels for Routed<'_> {
    fn model(&self, field: OcrField) -> &dyn ImageToText<RgbImage> {
        let routed = match self.group {
            None => true,
            Some(group) => Group::of(field) == Some(group),
        };
        self.models[if routed { self.candidate } else { BASELINE }].as_ref()
    }
}

#[derive(Default)]
struct Tally {
    correct: usize,
    total: usize,
}

struct Truth {
    relics: Vec<Value>,
    light_cones: Vec<Value>,
    characters: Vec<Value>,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dumps = PathBuf::from(args.next().unwrap_or_else(|| "debug_images".into()));
    let capture_path = args.next().expect("usage: ocr_field_eval <debug_images> <capture.json>");
    let expected_uid: Option<u64> = std::env::var("HSR_EVAL_UID").ok().and_then(|v| v.parse().ok());

    #[cfg(target_os = "windows")]
    if !genshin_scanner::cli::check_onnxruntime() {
        genshin_scanner::cli::download_onnxruntime().expect("onnxruntime download");
    }
    let models: Vec<Box<dyn ImageToText<RgbImage> + Send>> = MODELS
        .iter()
        .map(|name| genshin_scanner::scanner::common::ocr_factory::create_ocr_model(name).unwrap())
        .collect();
    let references = hsr_scanner::load_embedded_gilore_reference().unwrap();
    let capture: Value = serde_json::from_slice(&fs::read(&capture_path).unwrap()).unwrap();
    let truth = Truth {
        relics: capture["relics"].as_array().unwrap().clone(),
        light_cones: capture["light_cones"].as_array().unwrap().clone(),
        characters: capture["characters"].as_array().unwrap().clone(),
    };

    let trailblazer = std::env::var("HSR_SCAN_TRAILBLAZER_NAME").ok().map(|nickname| {
        hsr_scanner::TrailblazerIdentity {
            nickname,
            gender: hsr_scanner::TrailblazerGender::Stelle,
        }
    });

    let mut tallies: BTreeMap<(String, usize), Tally> = BTreeMap::new();
    let mut record = |label: String, model: usize, frame: &str, ok: bool| {
        if !ok {
            eprintln!("FAIL {label} {} {frame}", MODELS[model]);
        }
        let tally = tallies.entry((label, model)).or_default();
        tally.total += 1;
        tally.correct += usize::from(ok);
    };

    let rows: Vec<(Option<Group>, &str)> = vec![
        (None, "all fields"),
        (Some(Group::Uid), "uid"),
        (Some(Group::CharacterName), "character name"),
        (Some(Group::CharacterLevel), "character level"),
        (Some(Group::CharacterSkill), "trace levels"),
        (Some(Group::GearName), "relic name"),
        (Some(Group::GearLevel), "relic level"),
        (Some(Group::GearMainName), "relic main stat name"),
        (Some(Group::GearMainValue), "relic main stat value"),
        (Some(Group::GearSubName), "relic substat names"),
        (Some(Group::GearSubValue), "relic substat values"),
        (Some(Group::LightConeName), "light cone name"),
        (Some(Group::LightConeLevel), "light cone level"),
        (Some(Group::LightConeSuperimposition), "light cone superimposition"),
    ];

    let character_frames = frames(&dumps.join("characters"));
    let trace_frames = frames(&dumps.join("character_traces"));
    let gear_frames = frames(&dumps.join("gear"));
    let cone_frames = frames(&dumps.join("light_cones"));
    eprintln!(
        "frames: characters={} traces={} relics={} light_cones={}",
        character_frames.len(),
        trace_frames.len(),
        gear_frames.len(),
        cone_frames.len()
    );

    // Trace crops depend on the character's Path, and the capture entry on
    // its id; both come from the all-v4 parse.
    let mut identities: BTreeMap<String, (String, String)> = BTreeMap::new();

    for &(group, label) in &rows {
        for candidate in 0..MODELS.len() {
            let mut parser = PanelParser::new(PaddleOcrReader::with_models(Routed {
                models: &models,
                group,
                candidate,
            }));
            let applies = |wanted: &[Group]| group.is_none_or(|group| wanted.contains(&group));

            if applies(&[Group::Uid]) {
                if let Some(expected) = expected_uid {
                    for (name, frame) in character_frames.iter().chain(&gear_frames).take(20) {
                        let ok = parser.read_uid(frame).ok().flatten() == Some(expected);
                        record(format!("{label} [uid]"), candidate, name, ok);
                    }
                }
            }
            if applies(&[Group::CharacterName, Group::CharacterLevel]) {
                for (name, frame) in &character_frames {
                    let parsed =
                        parser.parse_character_details(frame, &references, trailblazer.as_ref(), 0);
                    let ok = parsed.as_ref().is_ok_and(|parsed| {
                        if candidate == BASELINE && group.is_none() {
                            identities.insert(
                                name.clone(),
                                (
                                    parsed.reference.path.clone(),
                                    parsed.observation.character_id.to_string(),
                                ),
                            );
                        }
                        truth.characters.iter().any(|entry| {
                            entry["id"] == parsed.observation.character_id.to_string()
                                && entry["level"] == parsed.observation.level
                                && entry["ascension"] == parsed.observation.ascension
                        })
                    });
                    record(format!("{label} [characters]"), candidate, name, ok);
                }
            }
            if applies(&[Group::CharacterSkill]) {
                for (name, frame) in &trace_frames {
                    let Some((path, id)) = identities.get(name) else { continue };
                    let Some(expected) = truth.characters.iter().find(|entry| entry["id"] == *id)
                    else {
                        continue;
                    };
                    let details = parser.parse_character_traces(frame, path);
                    let ok = details.as_ref().is_ok_and(|details| {
                        serde_json::to_value(&details.skills).unwrap() == expected["skills"]
                            && details
                                .memosprite
                                .as_ref()
                                .map(|memo| serde_json::to_value(memo).unwrap())
                                == expected.get("memosprite").cloned()
                    });
                    record(format!("{label} [traces]"), candidate, name, ok);
                    // Unlock nodes are pixel-classified, so one model suffices.
                    if candidate == BASELINE && group.is_none() {
                        let unlocks_ok = details.as_ref().is_ok_and(|details| {
                            details.traces.iter().all(|(key, unlocked)| {
                                expected["traces"].get(key).and_then(Value::as_bool)
                                    == Some(*unlocked)
                            })
                        });
                        record("trace unlocks".to_string(), candidate, name, unlocks_ok);
                    }
                }
            }
            if applies(&[
                Group::GearName,
                Group::GearLevel,
                Group::GearMainName,
                Group::GearMainValue,
                Group::GearSubName,
                Group::GearSubValue,
            ]) {
                for (name, frame) in &gear_frames {
                    let ok = parser
                        .discover_inventory_panel(frame, InventoryKind::Gear, &references)
                        .and_then(|layout| parser.parse_gear(frame, layout, &references))
                        .ok()
                        .is_some_and(|parsed| {
                            relic_in_capture(&parsed.observation, &references, &truth.relics)
                        });
                    record(format!("{label} [relics]"), candidate, name, ok);
                }
            }
            if applies(&[
                Group::LightConeName,
                Group::LightConeLevel,
                Group::LightConeSuperimposition,
            ]) {
                for (name, frame) in &cone_frames {
                    let ok = parser
                        .discover_inventory_panel(frame, InventoryKind::LightCone, &references)
                        .and_then(|layout| parser.parse_light_cone(frame, layout, &references))
                        .ok()
                        .is_some_and(|parsed| {
                            light_cone_in_capture(&parsed.observation, &truth.light_cones)
                        });
                    record(format!("{label} [light cones]"), candidate, name, ok);
                }
            }
        }
    }

    println!("| field (items) | {} |", MODELS.join(" | "));
    println!("|---|{}", "---|".repeat(MODELS.len()));
    let mut labels: Vec<String> = tallies.keys().map(|(label, _)| label.clone()).collect();
    labels.dedup();
    for label in labels {
        let cells: Vec<String> = (0..MODELS.len())
            .map(|model| {
                tallies
                    .get(&(label.clone(), model))
                    .map(|tally| format!("{}/{}", tally.correct, tally.total))
                    .unwrap_or_default()
            })
            .collect();
        println!("| {label} | {} |", cells.join(" | "));
    }
}

fn frames(dir: &Path) -> Vec<(String, RgbImage)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut frames: Vec<(String, RgbImage)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let image = image::open(entry.path().join("full.png")).ok()?.to_rgb8();
            Some((name, image))
        })
        .collect();
    frames.sort_by(|left, right| left.0.cmp(&right.0));
    frames
}

fn single_export(
    references: &ReferenceCache,
    gear: Vec<ObservedGear>,
    light_cones: Vec<ObservedLightCone>,
) -> Value {
    let snapshot = ObservationSnapshot {
        schema_version: hsr_scanner::OBSERVATION_SCHEMA_VERSION,
        evidence: ObservationEvidence {
            kind: EvidenceKind::ScreenCapture,
            revision: "ocr-field-eval".to_string(),
            coverage: InventoryCoverage {
                characters: CoverageLevel::Unknown,
                light_cones: CoverageLevel::Unknown,
                relics: CoverageLevel::Unknown,
            },
        },
        characters: Vec::new(),
        light_cones,
        gear,
    };
    build_scanner_export(&snapshot, references, &CaptureExportDetails::default(), None).unwrap()
}

/// HSR truncates displayed stats: percentages to 0.1, flat values to integers.
fn displayed(key: &str, value: f64) -> f64 {
    let scale = if key.ends_with('_') { 10.0 } else { 1.0 };
    ((value * scale) + 1e-6).floor() / scale
}

fn relic_in_capture(gear: &ObservedGear, references: &ReferenceCache, truth: &[Value]) -> bool {
    let export = single_export(references, vec![gear.clone()], Vec::new());
    let relic = &export["relics"][0];
    let substats = relic["substats"].as_array().unwrap();
    truth.iter().any(|candidate| {
        ["set_id", "slot", "rarity", "level", "mainstat"]
            .iter()
            .all(|key| candidate[key] == relic[key])
            && candidate["substats"].as_array().unwrap().len() == substats.len()
            && substats.iter().all(|sub| {
                candidate["substats"].as_array().unwrap().iter().any(|expected| {
                    expected["key"] == sub["key"]
                        && (displayed(
                            expected["key"].as_str().unwrap(),
                            expected["value"].as_f64().unwrap(),
                        ) - sub["value"].as_f64().unwrap())
                        .abs()
                            < 0.05
                })
            })
    })
}

fn light_cone_in_capture(cone: &ObservedLightCone, truth: &[Value]) -> bool {
    truth.iter().any(|candidate| {
        candidate["id"] == cone.light_cone_id.to_string()
            && candidate["level"] == cone.level
            && candidate["ascension"] == cone.ascension
            && candidate["superimposition"] == cone.superimposition
    })
}
