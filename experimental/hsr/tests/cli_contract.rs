#![cfg(feature = "experimental-hsr")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use hsr_scanner_experimental::manager::{HsrControllerLease, ManagerInstructionsEnvelope};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

static CONTROLLER_LEASE_TEST_SERIAL: Mutex<()> = Mutex::new(());

fn controller_lease_test_guard() -> std::sync::MutexGuard<'static, ()> {
    CONTROLLER_LEASE_TEST_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_HSRScannerExperimental")
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn unique_output(label: &str) -> TemporaryOutput {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "goodscanner-hsr-cli-{}-{}-{label}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    TemporaryOutput(path)
}

struct TemporaryOutput(PathBuf);

impl TemporaryOutput {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct TemporaryBundle(PathBuf);

impl TemporaryBundle {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryBundle {
    fn drop(&mut self) {
        let temporary_root = std::env::temp_dir();
        let is_owned_child = self.0.parent() == Some(temporary_root.as_path())
            && self
                .0
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("goodscanner-hsr-cli-complete-bundle-"));
        if is_owned_child {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn complete_test_reference_bundle() -> TemporaryBundle {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    const REVISION: &str = "014e33e2404f8cd668bf06fc2ea6db53b6bc3992";
    const REQUIRED_STATS: &[&str] = &[
        "HPDelta",
        "AttackDelta",
        "DefenceDelta",
        "SpeedDelta",
        "HPAddedRatio",
        "AttackAddedRatio",
        "DefenceAddedRatio",
        "CriticalChanceBase",
        "CriticalDamageBase",
        "HealRatioBase",
        "StatusProbabilityBase",
        "StatusResistanceBase",
        "BreakDamageAddedRatioBase",
        "SPRatioBase",
        "PhysicalAddedRatio",
        "FireAddedRatio",
        "IceAddedRatio",
        "ThunderAddedRatio",
        "WindAddedRatio",
        "QuantumAddedRatio",
        "ImaginaryAddedRatio",
    ];

    let root = std::env::temp_dir().join(format!(
        "goodscanner-hsr-cli-complete-bundle-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).expect("complete test-bundle directory must be new");

    let tiny_member = |filename: &str| -> Value {
        serde_json::from_slice(&fs::read(fixture(filename)).expect("tiny member must read"))
            .expect("tiny member must be JSON")
    };
    let localized = |en: String, zh_cn: String| {
        json!({
            "en": { "value": en },
            "zh-CN": { "value": zh_cn }
        })
    };
    let member = |collection: &str, value: Value| {
        json!({
            "bundle_id": "ggstarrail-reference",
            "collection": collection,
            "game_id": "honkai_star_rail",
            "schema_version": "1.1.0",
            "source_revision": REVISION,
            "value": value
        })
    };

    let mut characters = tiny_member("gilore_bundle/characters.json")["value"]
        .as_array()
        .expect("tiny characters")
        .clone();
    for index in 1..90_u32 {
        characters.push(json!({
            "id": (900_000 + index).to_string(),
            "name": localized(format!("Synthetic Character {index}"), format!("测试角色{index}")),
            "rarity": 4,
            "path_id": "Knight"
        }));
    }

    let mut light_cones = tiny_member("gilore_bundle/light_cones.json")["value"]
        .as_array()
        .expect("tiny light cones")
        .clone();
    for index in 1..160_u32 {
        light_cones.push(json!({
            "id": (800_000 + index).to_string(),
            "name": localized(format!("Synthetic Light Cone {index}"), format!("测试光锥{index}")),
            "rarity": 5,
            "path_id": "Knight"
        }));
    }

    let relic_sets = tiny_member("gilore_bundle/relic_sets.json")["value"].clone();
    let tiny_pieces = tiny_member("gilore_bundle/relic_pieces.json");
    let mut relic_pieces = vec![tiny_pieces["value"][0].clone()];
    let slots = ["HEAD", "HAND", "BODY", "FOOT", "NECK", "OBJECT"];
    for index in 1..700_u32 {
        let slot = slots[index as usize % slots.len()];
        let planar = matches!(slot, "NECK" | "OBJECT");
        relic_pieces.push(json!({
            "id": (700_000 + index).to_string(),
            "set_id": if planar { "301" } else { "101" },
            "slot": slot,
            "rarity": 5,
            "main_affix_group": 1_000 + index % 109,
            "max_level": 1,
            "icon_path": format!("Synthetic/Relic_{index}.png"),
            "name": localized(format!("Synthetic Relic {index}"), format!("测试遗器{index}"))
        }));
    }

    let tiny_properties = tiny_member("gilore_bundle/property_tables.json");
    let existing_properties = tiny_properties["value"]["properties"]
        .as_array()
        .expect("tiny properties");
    let mut properties = Vec::with_capacity(50);
    for key in REQUIRED_STATS {
        if let Some(existing) = existing_properties
            .iter()
            .find(|property| property["id"] == **key)
        {
            properties.push(existing.clone());
        } else {
            properties.push(json!({
                "id": key,
                "value_kind": if key.ends_with("Delta") { "flat" } else { "ratio" },
                "relic_name": localized(format!("Synthetic {key}"), format!("测试{key}"))
            }));
        }
    }
    while properties.len() < 50 {
        let index = properties.len();
        properties.push(json!({
            "id": format!("SyntheticProperty{index}"),
            "value_kind": "flat",
            "relic_name": localized(format!("Synthetic Property {index}"), format!("测试属性{index}"))
        }));
    }

    let tiny_progression = tiny_member("gilore_bundle/progression.json");
    let mut main_affixes = vec![tiny_progression["value"]["relic_main_affixes"][0].clone()];
    for index in 0..109_u32 {
        main_affixes.push(json!({
            "group_id": 1_000 + index,
            "property_id": REQUIRED_STATS[index as usize % REQUIRED_STATS.len()],
            "max_level": 1,
            "level_values": [1.0, 2.0]
        }));
    }

    let documents = [
        (
            "characters.json",
            member("characters", json!(characters)),
            90_u64,
        ),
        (
            "light_cones.json",
            member("light_cones", json!(light_cones)),
            160,
        ),
        ("relic_sets.json", member("relic_sets", relic_sets), 2),
        (
            "relic_pieces.json",
            member("relic_pieces", json!(relic_pieces)),
            700,
        ),
        (
            "property_tables.json",
            member("property_tables", json!({ "properties": properties })),
            50,
        ),
        (
            "progression.json",
            member("progression", json!({ "relic_main_affixes": main_affixes })),
            110,
        ),
    ];
    let mut files = Map::new();
    for (filename, document, entity_count) in documents {
        let mut bytes = serde_json::to_vec_pretty(&document).expect("member must serialize");
        bytes.push(b'\n');
        fs::write(root.join(filename), &bytes).expect("member must write");
        files.insert(
            filename.to_owned(),
            json!({
                "byte_count": bytes.len(),
                "entity_count": entity_count,
                "sha256": format!("{:x}", Sha256::digest(&bytes))
            }),
        );
    }
    let manifest = json!({
        "bundle_id": "ggstarrail-reference",
        "game_id": "honkai_star_rail",
        "schema_version": "1.1.0",
        "locales": ["en", "zh-CN"],
        "source": {
            "source_id": "turn_based_game_data",
            "revision": REVISION
        },
        "files": files
    });
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest must serialize");
    manifest_bytes.push(b'\n');
    fs::write(root.join("manifest.json"), manifest_bytes).expect("manifest must write");
    TemporaryBundle(root)
}

#[test]
fn executable_exposes_all_isolated_application_flows() {
    let output = Command::new(binary())
        .args(["--lang", "en", "--help"])
        .output()
        .expect("experimental executable must start");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("help must be UTF-8");
    for command in [
        "fixture-export",
        "scan",
        "capture",
        "import-capture",
        "manager",
    ] {
        assert!(help.contains(command), "missing command {command}");
    }
    assert!(help.contains("fully isolated from official GOODScanner/GOODCapture"));
    assert!(help.contains("实时变更需要单独运行 apply"));
}

#[test]
fn fixture_export_runs_through_the_production_cli_and_never_overwrites() {
    let output = unique_output("fixture");
    let first = Command::new(binary())
        .args(["--lang", "en", "fixture-export", "--reference-bundle"])
        .arg(fixture("gilore_bundle"))
        .arg("--input")
        .arg(fixture("observations.json"))
        .arg("--output")
        .arg(output.path())
        .output()
        .expect("fixture command must start");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );

    let value: Value = serde_json::from_slice(&fs::read(output.path()).expect("output must exist"))
        .expect("output must be JSON");
    assert_eq!(value["schema"], "goodscanner.hsr.experimental");
    assert_eq!(value["schemaVersion"], 2);
    assert_eq!(value["source"]["kind"], "sanitizedFixture");
    assert_eq!(value["privacy"]["accountIdentifiersIncluded"], false);
    assert_eq!(value["privacy"]["rawPacketDataIncluded"], false);
    assert_eq!(value["privacy"]["serverItemIdentifiersIncluded"], false);

    let second = Command::new(binary())
        .args(["--lang", "en", "fixture-export", "--reference-bundle"])
        .arg(fixture("gilore_bundle"))
        .arg("--input")
        .arg(fixture("observations.json"))
        .arg("--output")
        .arg(output.path())
        .output()
        .expect("second fixture command must start");
    assert!(
        !second.status.success(),
        "existing output must never be overwritten"
    );
    assert!(String::from_utf8_lossy(&second.stderr).contains("HSR-EXPORT-CREATE"));
}

#[test]
fn manager_apply_refuses_before_file_or_device_access_without_separate_confirmation() {
    let journal = unique_output("must-not-exist");
    let output = Command::new(binary())
        .args([
            "--lang",
            "en",
            "manager",
            "apply",
            "--reference-bundle",
            "missing-reference-bundle",
            "--instructions",
            "missing-instructions.json",
            "--confirm-digest",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--journal",
        ])
        .arg(journal.path())
        .arg("--allow-lock")
        .output()
        .expect("manager command must start");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("separate live-mutation confirmation is missing"));
    assert!(stderr.contains("HSR_MANAGER_EXPLICIT_CONFIRMATION_REQUIRED"));
    assert!(!journal.path().exists());
}

#[test]
fn manager_cli_rejects_stale_matcher_semantics_before_live_device_creation() {
    let _serial = controller_lease_test_guard();
    let complete_bundle = complete_test_reference_bundle();
    let instructions = unique_output("stale-manager");
    let source = fs::read_to_string(fixture("manager_instructions_v1.json"))
        .expect("manager fixture must exist");
    let mut envelope = ManagerInstructionsEnvelope::parse_json(&source)
        .expect("manager fixture must satisfy its semantic digest");
    envelope.instructions[0].matcher.main_stat.value = 999.0;
    envelope.idempotency_key = envelope
        .expected_idempotency_key()
        .expect("production canonicalizer must recompute the mutated fixture digest");
    fs::write(
        instructions.path(),
        serde_json::to_vec_pretty(&envelope).expect("manager fixture must serialize"),
    )
    .expect("temporary manager fixture must be writable");

    let output = Command::new(binary())
        .args(["--lang", "en", "manager", "preview", "--reference-bundle"])
        .arg(complete_bundle.path())
        .arg("--instructions")
        .arg(instructions.path())
        .output()
        .expect("manager preview command must start");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("HSR_MANAGER_REFERENCE_MISMATCH"),
        "{stderr}"
    );
    assert!(
        stderr.contains("not a canonical visible-equivalence reference"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("game window"),
        "must fail before device access"
    );
}

#[test]
fn tiny_fixture_reference_is_rejected_by_every_live_scanner_and_manager_path() {
    let _serial = controller_lease_test_guard();
    let scan_output = unique_output("tiny-live-scan");
    let journal = unique_output("tiny-live-manager-journal");
    let digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    let scan = Command::new(binary())
        .args(["--lang", "en", "scan", "--reference-bundle"])
        .arg(fixture("gilore_bundle"))
        .arg("--output")
        .arg(scan_output.path())
        .output()
        .expect("scan command must start");
    let preview = Command::new(binary())
        .args(["--lang", "en", "manager", "preview", "--reference-bundle"])
        .arg(fixture("gilore_bundle"))
        .arg("--instructions")
        .arg(fixture("manager_instructions_v1.json"))
        .output()
        .expect("manager preview command must start");
    let apply = Command::new(binary())
        .args(["--lang", "en", "manager", "apply", "--reference-bundle"])
        .arg(fixture("gilore_bundle"))
        .arg("--instructions")
        .arg(fixture("manager_instructions_v1.json"))
        .arg("--confirm-digest")
        .arg(digest)
        .arg("--journal")
        .arg(journal.path())
        .args(["--confirm-live-mutation", "--allow-lock"])
        .output()
        .expect("manager apply command must start");

    for (label, output) in [
        ("scan", scan),
        ("manager preview", preview),
        ("manager apply", apply),
    ] {
        assert!(
            !output.status.success(),
            "{label} accepted a tiny test bundle"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("HSR-REF-LIVE-INCOMPLETE"),
            "{label}: {stderr}"
        );
        assert!(
            !stderr.contains("game window"),
            "{label} reached device setup: {stderr}"
        );
    }
    assert!(!scan_output.path().exists());
    assert!(!journal.path().exists());
}

#[test]
fn all_game_driving_commands_contend_on_one_controller_lease_across_journals() {
    let _serial = controller_lease_test_guard();
    let _lease = HsrControllerLease::try_acquire().expect("test must own the controller lease");
    let missing_reference = unique_output("missing-contention-reference");
    let missing_instructions = unique_output("missing-contention-instructions");
    let scan_output = unique_output("blocked-scan-output");
    let first_journal = unique_output("blocked-first-journal");
    let second_journal = unique_output("blocked-second-journal");
    let digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    let scan = Command::new(binary())
        .args(["--lang", "en", "scan", "--reference-bundle"])
        .arg(missing_reference.path())
        .arg("--output")
        .arg(scan_output.path())
        .output()
        .expect("scan contention probe must start");
    let preview = Command::new(binary())
        .args(["--lang", "en", "manager", "preview", "--reference-bundle"])
        .arg(missing_reference.path())
        .arg("--instructions")
        .arg(missing_instructions.path())
        .output()
        .expect("preview contention probe must start");

    let run_apply = |journal: &Path| {
        Command::new(binary())
            .args(["--lang", "en", "manager", "apply", "--reference-bundle"])
            .arg(missing_reference.path())
            .arg("--instructions")
            .arg(missing_instructions.path())
            .arg("--confirm-digest")
            .arg(digest)
            .arg("--journal")
            .arg(journal)
            .args(["--confirm-live-mutation", "--allow-lock"])
            .output()
            .expect("apply contention probe must start")
    };
    let first_apply = run_apply(first_journal.path());
    let second_apply = run_apply(second_journal.path());

    for (label, output) in [
        ("scan", scan),
        ("manager preview", preview),
        ("manager apply journal one", first_apply),
        ("manager apply journal two", second_apply),
    ] {
        assert!(
            !output.status.success(),
            "{label} bypassed the controller lease"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("HSR_CONTROLLER_BUSY"), "{label}: {stderr}");
        assert!(
            !stderr.contains("HSR-GILORE-READ") && !stderr.contains("game window"),
            "{label} performed reference/device work before lease rejection: {stderr}"
        );
    }
    assert!(!first_journal.path().exists());
    assert!(!second_journal.path().exists());
}

#[test]
fn offline_fixture_and_capture_import_do_not_require_the_controller_lease() {
    let _serial = controller_lease_test_guard();
    let _lease = HsrControllerLease::try_acquire().expect("test must own the controller lease");
    let fixture_output = unique_output("offline-fixture-under-lease");
    let capture_output = unique_output("offline-capture-under-lease");

    let fixture_export = Command::new(binary())
        .args(["--lang", "en", "fixture-export", "--reference-bundle"])
        .arg(fixture("gilore_bundle"))
        .arg("--input")
        .arg(fixture("observations.json"))
        .arg("--output")
        .arg(fixture_output.path())
        .output()
        .expect("offline fixture export must start");
    assert!(
        fixture_export.status.success(),
        "{}",
        String::from_utf8_lossy(&fixture_export.stderr)
    );

    let capture_import = Command::new(binary())
        .args(["--lang", "en", "import-capture", "--reference-bundle"])
        .arg(fixture("gilore_bundle"))
        .arg("--input")
        .arg(fixture("capture_reliquary_archive.json"))
        .arg("--output")
        .arg(capture_output.path())
        .output()
        .expect("offline capture import must start");
    assert!(
        capture_import.status.success(),
        "{}",
        String::from_utf8_lossy(&capture_import.stderr)
    );

    assert!(fixture_output.path().exists());
    assert!(capture_output.path().exists());
}
