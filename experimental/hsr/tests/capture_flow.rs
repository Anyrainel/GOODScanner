use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Mutex,
    },
};

use hsr_scanner_experimental::{
    build_export,
    capture::{
        import_reliquary_archive_file, parse_reliquary_archive, ArchiverInvocation,
        ArchiverProcessResult, ArchiverProcessRunner, PinnedReliquaryArchiver,
    },
    localization::BilingualName,
    reference::{GiloreBundleReferenceProvider, ReferenceProvider},
    CoverageLevel, GearSlot, Language, ReferenceCache, RelicMainAffixReference, StatReference,
    StatValueKind,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn references() -> ReferenceCache {
    let provider = GiloreBundleReferenceProvider::new(fixture("gilore_bundle"));
    let mut snapshot = provider.load().expect("verified GIlore fixture must load");

    while snapshot.characters.len() < 90 {
        let index = snapshot.characters.len() as u32;
        let mut entry = snapshot.characters[0].clone();
        entry.game_id = 1_000_000 + index;
        entry.key = entry.game_id.to_string();
        entry.name = synthetic_name("Character", index);
        snapshot.characters.push(entry);
    }
    while snapshot.light_cones.len() < 160 {
        let index = snapshot.light_cones.len() as u32;
        let mut entry = snapshot.light_cones[0].clone();
        entry.game_id = 2_000_000 + index;
        entry.key = entry.game_id.to_string();
        entry.name = synthetic_name("LightCone", index);
        snapshot.light_cones.push(entry);
    }

    const MAIN_PROPERTIES: &[(&str, StatValueKind)] = &[
        ("HPDelta", StatValueKind::Flat),
        ("AttackDelta", StatValueKind::Flat),
        ("SpeedDelta", StatValueKind::Flat),
        ("HPAddedRatio", StatValueKind::Ratio),
        ("AttackAddedRatio", StatValueKind::Ratio),
        ("DefenceAddedRatio", StatValueKind::Ratio),
        ("CriticalChanceBase", StatValueKind::Ratio),
        ("CriticalDamageBase", StatValueKind::Ratio),
        ("HealRatioBase", StatValueKind::Ratio),
        ("StatusProbabilityBase", StatValueKind::Ratio),
        ("BreakDamageAddedRatioBase", StatValueKind::Ratio),
        ("SPRatioBase", StatValueKind::Ratio),
        ("PhysicalAddedRatio", StatValueKind::Ratio),
        ("FireAddedRatio", StatValueKind::Ratio),
        ("IceAddedRatio", StatValueKind::Ratio),
        ("ThunderAddedRatio", StatValueKind::Ratio),
        ("WindAddedRatio", StatValueKind::Ratio),
        ("QuantumAddedRatio", StatValueKind::Ratio),
        ("ImaginaryAddedRatio", StatValueKind::Ratio),
    ];
    const EXTRA_PROPERTIES: &[(&str, StatValueKind)] = &[
        ("DefenceDelta", StatValueKind::Flat),
        ("StatusResistanceBase", StatValueKind::Ratio),
    ];
    for (key, kind) in MAIN_PROPERTIES.iter().chain(EXTRA_PROPERTIES) {
        if snapshot.stats.iter().all(|entry| entry.key != *key) {
            snapshot.stats.push(StatReference {
                key: (*key).to_string(),
                name: synthetic_name(key, 0),
                value_kind: *kind,
            });
        }
    }
    while snapshot.stats.len() < 50 {
        let index = snapshot.stats.len() as u32;
        snapshot.stats.push(StatReference {
            key: format!("SyntheticProperty{index}"),
            name: synthetic_name("Property", index),
            value_kind: StatValueKind::Flat,
        });
    }

    let mut next_group = 100_000_u32;
    for (property, _) in MAIN_PROPERTIES {
        if snapshot
            .relic_main_affixes
            .iter()
            .all(|entry| entry.property_id != *property)
        {
            snapshot
                .relic_main_affixes
                .push(synthetic_affix(next_group, property));
            snapshot
                .gear_pieces
                .push(synthetic_gear(3_000_000 + next_group, next_group, property));
            next_group += 1;
        }
    }
    while snapshot.relic_main_affixes.len() < 110 {
        snapshot
            .relic_main_affixes
            .push(synthetic_affix(next_group, "HPDelta"));
        snapshot.gear_pieces.push(synthetic_gear(
            3_000_000 + next_group,
            next_group,
            "HPDelta",
        ));
        next_group += 1;
    }
    while snapshot.gear_pieces.len() < 700 {
        let index = snapshot.gear_pieces.len() as u32;
        let mut entry = snapshot.gear_pieces[0].clone();
        entry.game_id = 4_000_000 + index;
        entry.key = entry.game_id.to_string();
        entry.name = synthetic_name("Gear", index);
        entry.icon_path = format!("Synthetic/Icon/{index}.png");
        entry.set_key = format!("SyntheticSet{index}");
        entry.set_name = synthetic_name("Set", index);
        snapshot.gear_pieces.push(entry);
    }

    let cache = ReferenceCache::from_snapshot(snapshot).expect("synthetic live profile must load");
    cache
        .validate_live_complete_profile()
        .expect("synthetic live profile must meet production floors");
    cache
}

fn tiny_references() -> ReferenceCache {
    ReferenceCache::from_provider(&GiloreBundleReferenceProvider::new(fixture(
        "gilore_bundle",
    )))
    .expect("tiny verified GIlore fixture must load")
}

fn synthetic_name(label: &str, index: u32) -> BilingualName {
    BilingualName {
        zh_cn: format!("测试{label}{index}"),
        en: format!("Test {label} {index}"),
    }
}

fn synthetic_affix(group_id: u32, property_id: &str) -> RelicMainAffixReference {
    RelicMainAffixReference {
        group_id,
        property_id: property_id.to_string(),
        max_level: 15,
        level_values: vec![0.1; 16],
    }
}

fn synthetic_gear(
    game_id: u32,
    group_id: u32,
    property: &str,
) -> hsr_scanner_experimental::GearReference {
    let slot = match property {
        "HPDelta" => GearSlot::Head,
        "AttackDelta" => GearSlot::Hands,
        "SpeedDelta" => GearSlot::Feet,
        "SPRatioBase" => GearSlot::LinkRope,
        _ => GearSlot::Body,
    };
    hsr_scanner_experimental::GearReference {
        game_id,
        key: game_id.to_string(),
        name: synthetic_name("Gear", game_id),
        icon_path: format!("Synthetic/Icon/{game_id}.png"),
        set_key: format!("SyntheticSet{game_id}"),
        set_name: synthetic_name("Set", game_id),
        rarity: 5,
        category: slot.category(),
        slot,
        main_affix_group: group_id,
        max_level: 15,
    }
}

fn archive_bytes() -> Vec<u8> {
    fs::read(fixture("capture_reliquary_archive.json")).expect("capture fixture must be readable")
}

fn archive_value() -> Value {
    serde_json::from_slice(&archive_bytes()).expect("capture fixture must be JSON")
}

#[test]
fn offline_archive_normalizes_to_the_v2_account_golden() {
    let references = references();
    let imported =
        import_reliquary_archive_file(fixture("capture_reliquary_archive.json"), &references)
            .expect("audited archive must import");
    assert_eq!(imported.coverage().characters, CoverageLevel::Complete);
    assert_eq!(imported.coverage().light_cones, CoverageLevel::Complete);
    assert_eq!(imported.coverage().relics, CoverageLevel::Complete);

    let export = build_export(imported.into_observations(), &references)
        .expect("normalized packet observation must export");
    let actual = serde_json::to_value(export).expect("export must serialize");
    let expected: Value = serde_json::from_slice(
        &fs::read(fixture("capture_expected_export.json"))
            .expect("capture golden must be readable"),
    )
    .expect("capture golden must be JSON");
    assert_eq!(actual, expected);
    assert_eq!(actual["source"]["kind"], "packetCapture");
    assert_eq!(actual["planarOrnaments"][0]["mainStat"]["value"], 43.2);
    assert_eq!(actual["privacy"]["accountIdentifiersIncluded"], false);
    assert_eq!(actual["privacy"]["rawPacketDataIncluded"], false);
    assert_eq!(actual["privacy"]["serverItemIdentifiersIncluded"], false);
}

#[test]
fn account_and_instance_identifiers_are_discarded_before_normalization() {
    let references = references();
    let mut archive = archive_value();
    archive["metadata"]["uid"] = Value::from("account-secret-sentinel");
    archive["metadata"]["nickname"] = Value::from("nickname-secret-sentinel");
    archive["light_cones"][0]["_uid"] = Value::from("lightcone-secret-sentinel");
    archive["light_cones"][0]["serverItemId"] = Value::from("server-secret-sentinel");
    archive["relics"][0]["_uid"] = Value::from("relic-secret-sentinel");
    archive["relics"][0]["guid"] = Value::from("guid-secret-sentinel");

    let imported = parse_reliquary_archive(&serde_json::to_vec(&archive).unwrap(), &references)
        .expect("known identifiers must be discarded, not retained");
    let export = build_export(imported.into_observations(), &references)
        .expect("redacted archive must export");
    let exported_value = serde_json::to_value(&export).unwrap();
    assert_no_exact_keys(
        &exported_value,
        &["uid", "_uid", "guid", "serverItemId", "nickname"],
    );
    assert_eq!(
        exported_value["privacy"]["serverItemIdentifiersIncluded"],
        false
    );
    let serialized = serde_json::to_string(&exported_value).unwrap();
    for sentinel in [
        "account-secret-sentinel",
        "nickname-secret-sentinel",
        "lightcone-secret-sentinel",
        "server-secret-sentinel",
        "relic-secret-sentinel",
        "guid-secret-sentinel",
    ] {
        assert!(!serialized.contains(sentinel));
    }
}

#[test]
fn embedded_raw_packet_data_is_rejected_without_echoing_its_value() {
    let references = references();
    let mut archive = archive_value();
    archive["rawPacketData"] = Value::from("raw-packet-secret-sentinel");
    let error = parse_reliquary_archive(&serde_json::to_vec(&archive).unwrap(), &references)
        .expect_err("raw packet payloads must never cross the import boundary");
    assert_eq!(error.code(), "HSR-CAPTURE-RAW-PACKET");
    let english = error.localized_message(Language::En);
    let chinese = error.localized_message(Language::ZhCn);
    assert!(english.starts_with("The input contains a prohibited account or session field"));
    assert!(chinese.starts_with("输入包含禁止保留的账号或会话字段"));
    assert!(!english.contains("raw-packet-secret-sentinel"));
    assert!(!chinese.contains("raw-packet-secret-sentinel"));
}

#[test]
fn credentials_are_rejected_instead_of_silently_accepted() {
    let references = references();
    let mut archive = archive_value();
    archive["sessionToken"] = Value::from("credential-secret-sentinel");
    let error = parse_reliquary_archive(&serde_json::to_vec(&archive).unwrap(), &references)
        .expect_err("credentials are not part of the archive contract");
    assert_eq!(error.code(), "HSR-DATA-SENSITIVE");
    assert!(!error
        .localized_message(Language::En)
        .contains("credential-secret-sentinel"));
    assert!(!error
        .localized_message(Language::ZhCn)
        .contains("credential-secret-sentinel"));
}

#[test]
fn source_build_and_format_are_pinned_fail_closed() {
    let references = references();
    for (field, replacement, code) in [
        ("source", Value::from("HSR-Scanner"), "HSR-CAPTURE-SOURCE"),
        ("build", Value::from("0.18.1"), "HSR-CAPTURE-BUILD"),
        ("version", Value::from(5), "HSR-CAPTURE-VERSION"),
    ] {
        let mut archive = archive_value();
        archive[field] = replacement;
        let error = parse_reliquary_archive(&serde_json::to_vec(&archive).unwrap(), &references)
            .expect_err("unreviewed helper contracts must fail closed");
        assert_eq!(error.code(), code);
        assert!(error
            .localized_message(Language::En)
            .starts_with("The HSR observation data is invalid"));
        assert!(error
            .localized_message(Language::ZhCn)
            .starts_with("HSR 观测数据无效"));
    }
}

#[test]
fn partial_offline_archives_are_truthful_but_live_capture_rejects_them() {
    let references = references();
    let mut archive = archive_value();
    archive["light_cones"] = Value::Array(Vec::new());
    let bytes = serde_json::to_vec(&archive).unwrap();

    let offline = parse_reliquary_archive(&bytes, &references)
        .expect("offline partial imports retain explicit unknown coverage");
    assert_eq!(offline.coverage().light_cones, CoverageLevel::Unknown);
    assert_eq!(offline.coverage().characters, CoverageLevel::Complete);
    assert_eq!(offline.coverage().relics, CoverageLevel::Complete);

    let test_dir = TestDirectory::create("partial-live");
    let helper_bytes = b"fake checksum-pinned helper";
    let helper_path = test_dir.path().join("reliquary-archiver.exe");
    fs::write(&helper_path, helper_bytes).unwrap();
    let pin = PinnedReliquaryArchiver::new(&helper_path, sha256_hex(helper_bytes)).unwrap();
    let runner = RecordingRunner::new(bytes);
    let error = pin
        .capture_with_runner(&runner, &references)
        .expect_err("a live helper run cannot claim partial output as complete");
    assert_eq!(error.code(), "HSR-CAPTURE-INCOMPLETE");
}

#[test]
fn helper_runner_uses_only_attended_one_shot_flags_and_private_output() {
    let references = references();
    let test_dir = TestDirectory::create("runner");
    let helper_bytes = b"fake checksum-pinned helper";
    let helper_path = test_dir.path().join("reliquary-archiver.exe");
    fs::write(&helper_path, helper_bytes).unwrap();
    let pin = PinnedReliquaryArchiver::new(&helper_path, sha256_hex(helper_bytes))
        .unwrap()
        .with_timeout_seconds(17)
        .unwrap();
    let runner = RecordingRunner::new(archive_bytes());

    let imported = pin
        .capture_with_runner(&runner, &references)
        .expect("fake process must exercise the production invocation boundary");
    assert_eq!(imported.coverage().characters, CoverageLevel::Complete);
    assert_eq!(runner.calls.load(Ordering::SeqCst), 1);

    let invocation = runner
        .invocation
        .lock()
        .unwrap()
        .clone()
        .expect("runner must receive an invocation");
    assert_ne!(invocation.executable(), helper_path);
    assert_eq!(
        invocation.executable().parent(),
        Some(invocation.working_directory())
    );
    assert_eq!(
        invocation
            .executable()
            .file_name()
            .and_then(|name| name.to_str()),
        Some("verified-reliquary-archiver.exe")
    );
    assert_eq!(invocation.maximum_runtime().as_secs(), 47);
    let arguments: Vec<String> = invocation
        .arguments()
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        &arguments[..5],
        [
            "--no-update",
            "--exit-after-capture",
            "--headless",
            "--timeout",
            "17"
        ]
    );
    assert_eq!(Path::new(&arguments[5]), invocation.output_path());
    assert_eq!(
        invocation.output_path().parent(),
        Some(invocation.working_directory())
    );
    assert!(!arguments.iter().any(|value| value == "--stream"));
    assert!(!arguments.iter().any(|value| value == "--log-path"));
    assert!(!invocation.output_path().exists());
    assert!(!invocation.working_directory().exists());
}

#[test]
fn failed_helper_removes_ancillary_files_from_its_private_working_directory() {
    let references = references();
    let test_dir = TestDirectory::create("ancillary-cleanup");
    let helper_bytes = b"fake checksum-pinned helper";
    let helper_path = test_dir.path().join("reliquary-archiver.exe");
    fs::write(&helper_path, helper_bytes).unwrap();
    let pin = PinnedReliquaryArchiver::new(&helper_path, sha256_hex(helper_bytes)).unwrap();
    let runner = AncillaryFailureRunner::default();

    let error = pin
        .capture_with_runner(&runner, &references)
        .expect_err("a failed helper must not be imported");
    assert_eq!(error.code(), "HSR-CAPTURE-EXIT");

    let invocation = runner
        .invocation
        .lock()
        .unwrap()
        .clone()
        .expect("runner must receive an invocation");
    assert!(!invocation.output_path().exists());
    assert!(!invocation.working_directory().exists());
}

#[test]
fn helper_checksum_is_verified_before_any_process_is_started() {
    let references = references();
    let test_dir = TestDirectory::create("checksum");
    let helper_path = test_dir.path().join("reliquary-archiver.exe");
    fs::write(&helper_path, b"unexpected helper bytes").unwrap();
    let pin = PinnedReliquaryArchiver::new(&helper_path, "00".repeat(32)).unwrap();
    let runner = RecordingRunner::new(archive_bytes());
    let error = pin
        .capture_with_runner(&runner, &references)
        .expect_err("a mismatched helper must not run");
    assert_eq!(error.code(), "HSR-CAPTURE-CHECKSUM");
    assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn tiny_reference_is_rejected_before_helper_inspection_or_process_start() {
    let references = tiny_references();
    let runner = RecordingRunner::new(archive_bytes());
    let pin =
        PinnedReliquaryArchiver::new(fixture("helper-that-does-not-exist.exe"), "00".repeat(32))
            .unwrap();

    let error = pin
        .capture_with_runner(&runner, &references)
        .expect_err("tiny fixture must fail before helper or account interaction");
    assert_eq!(error.code(), "HSR-REF-LIVE-INCOMPLETE");
    assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn tiny_verified_reference_remains_valid_for_offline_import() {
    let imported = parse_reliquary_archive(&archive_bytes(), &tiny_references())
        .expect("offline fixture import should not require a production-sized reference bundle");
    assert_eq!(imported.coverage().characters, CoverageLevel::Complete);
    assert_eq!(imported.coverage().light_cones, CoverageLevel::Complete);
    assert_eq!(imported.coverage().relics, CoverageLevel::Complete);
}

#[test]
fn original_helper_replacement_cannot_change_the_invoked_verified_copy() {
    let references = references();
    let test_dir = TestDirectory::create("helper-race");
    let approved = b"approved checksum-pinned helper bytes".to_vec();
    let original = test_dir.path().join("reliquary-archiver.exe");
    fs::write(&original, &approved).unwrap();
    let pin = PinnedReliquaryArchiver::new(&original, sha256_hex(&approved)).unwrap();
    let runner = ReplacingOriginalRunner::new(original.clone(), approved, archive_bytes());

    pin.capture_with_runner(&runner, &references)
        .expect("the staged approved image must be independent of the replaced original");
    assert_eq!(
        fs::read(&original).expect("replacement must remain at user path"),
        b"unapproved replacement"
    );
    let invocation = runner
        .invocation
        .lock()
        .unwrap()
        .clone()
        .expect("runner invocation");
    assert_ne!(invocation.executable(), original);
    assert!(!invocation.working_directory().exists());
}

#[test]
fn explicit_cleanup_failure_is_surfaced_and_drop_remains_fallback() {
    let references = references();
    let test_dir = TestDirectory::create("cleanup-failure");
    let approved = b"approved checksum-pinned helper bytes";
    let original = test_dir.path().join("reliquary-archiver.exe");
    fs::write(&original, approved).unwrap();
    let pin = PinnedReliquaryArchiver::new(&original, sha256_hex(approved)).unwrap();
    let runner = RecordingRunner::new(archive_bytes());

    let error = pin
        .capture_with_runner_and_cleanup_for_test(&runner, &references, |_path| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "synthetic cleanup denial",
            ))
        })
        .expect_err("cleanup uncertainty must override an otherwise successful capture");
    assert_eq!(error.code(), "HSR-CAPTURE-CLEANUP");
    let english = error.localized_message(Language::En);
    assert!(english.contains("goodscanner-hsr-capture-"));
    assert!(!english.contains("Moment of Victory"));
    let invocation = runner
        .invocation
        .lock()
        .unwrap()
        .clone()
        .expect("runner invocation");
    assert!(
        !invocation.working_directory().exists(),
        "Drop must make one contained best-effort fallback attempt"
    );
}

#[test]
fn normalized_output_is_independent_of_archive_order_and_instance_ids() {
    let references = references();
    let baseline = parse_reliquary_archive(&archive_bytes(), &references)
        .expect("baseline archive must import");
    let baseline =
        serde_json::to_value(build_export(baseline.into_observations(), &references).unwrap())
            .unwrap();

    let mut shuffled = archive_value();
    shuffled["relics"].as_array_mut().unwrap().reverse();
    for (index, relic) in shuffled["relics"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        relic["_uid"] = Value::from(format!("different-instance-{index}"));
        relic["substats"].as_array_mut().unwrap().reverse();
    }
    shuffled["light_cones"][0]["_uid"] = Value::from("different-light-cone-instance");
    shuffled["metadata"]["uid"] = Value::from("different-account");
    let shuffled = parse_reliquary_archive(&serde_json::to_vec(&shuffled).unwrap(), &references)
        .expect("shuffled archive must import");
    let shuffled =
        serde_json::to_value(build_export(shuffled.into_observations(), &references).unwrap())
            .unwrap();
    assert_eq!(shuffled, baseline);
}

#[test]
fn duplicate_four_star_relic_buckets_resolve_from_main_affix_evidence() {
    let references = references();
    for (mainstat, expected_value) in [("ATK", 28.7544), ("Outgoing Healing Boost", 23.0033)] {
        let mut archive = archive_value();
        let mut body = archive["relics"][1].clone();
        body["slot"] = Value::from("Body");
        body["rarity"] = Value::from(4);
        body["level"] = Value::from(12);
        body["mainstat"] = Value::from(mainstat);
        body["substats"] = Value::Array(Vec::new());
        archive["relics"] = Value::Array(vec![body]);

        let imported = parse_reliquary_archive(&serde_json::to_vec(&archive).unwrap(), &references)
            .expect("main-affix evidence must resolve a real duplicate four-star bucket");
        let export = build_export(imported.into_observations(), &references)
            .expect("canonical duplicate must export");
        assert_eq!(export.relics.len(), 1);
        assert_eq!(export.relics[0].gear.game_id, 51013);
        assert!((export.relics[0].gear.main_stat.value - expected_value).abs() < 1e-9);
    }
}

#[test]
fn capture_boundary_has_no_packet_write_or_live_stream_surface() {
    let source = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("capture.rs"),
    )
    .expect("capture module must be readable");
    for forbidden in [
        "UdpSocket",
        "TcpStream",
        "LockRelicCsReq",
        "DiscardRelicCsReq",
        "OsString::from(\"--stream\")",
        "OsString::from(\"--pcap\")",
        "OsString::from(\"--etl\")",
    ] {
        assert!(
            !source.contains(forbidden),
            "capture boundary gained a prohibited surface: {forbidden}"
        );
    }
    assert!(source.contains(".stdout(Stdio::null())"));
    assert!(source.contains(".stderr(Stdio::null())"));
}

struct RecordingRunner {
    archive: Vec<u8>,
    invocation: Mutex<Option<ArchiverInvocation>>,
    calls: AtomicUsize,
}

impl RecordingRunner {
    fn new(archive: Vec<u8>) -> Self {
        Self {
            archive,
            invocation: Mutex::new(None),
            calls: AtomicUsize::new(0),
        }
    }
}

impl ArchiverProcessRunner for RecordingRunner {
    fn run(&self, invocation: &ArchiverInvocation) -> io::Result<ArchiverProcessResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(!invocation.output_path().exists());
        assert!(invocation.working_directory().is_dir());
        *self.invocation.lock().unwrap() = Some(invocation.clone());
        fs::write(invocation.output_path(), &self.archive)?;
        Ok(ArchiverProcessResult::success())
    }
}

struct ReplacingOriginalRunner {
    original: PathBuf,
    approved: Vec<u8>,
    archive: Vec<u8>,
    invocation: Mutex<Option<ArchiverInvocation>>,
}

impl ReplacingOriginalRunner {
    fn new(original: PathBuf, approved: Vec<u8>, archive: Vec<u8>) -> Self {
        Self {
            original,
            approved,
            archive,
            invocation: Mutex::new(None),
        }
    }
}

impl ArchiverProcessRunner for ReplacingOriginalRunner {
    fn run(&self, invocation: &ArchiverInvocation) -> io::Result<ArchiverProcessResult> {
        fs::write(&self.original, b"unapproved replacement")?;
        if fs::read(invocation.executable())? != self.approved {
            return Err(io::Error::other(
                "runner was not given the checksum-verified staged bytes",
            ));
        }
        *self.invocation.lock().unwrap() = Some(invocation.clone());
        fs::write(invocation.output_path(), &self.archive)?;
        Ok(ArchiverProcessResult::success())
    }
}

#[derive(Default)]
struct AncillaryFailureRunner {
    invocation: Mutex<Option<ArchiverInvocation>>,
}

impl ArchiverProcessRunner for AncillaryFailureRunner {
    fn run(&self, invocation: &ArchiverInvocation) -> io::Result<ArchiverProcessResult> {
        *self.invocation.lock().unwrap() = Some(invocation.clone());
        fs::write(
            invocation.working_directory().join("crashlog.txt"),
            b"synthetic helper crash output",
        )?;
        Ok(ArchiverProcessResult::failure(Some(101)))
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn assert_no_exact_keys(value: &Value, prohibited: &[&str]) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                assert!(
                    !prohibited.contains(&key.as_str()),
                    "prohibited field survived normalization: {key}"
                );
                assert_no_exact_keys(child, prohibited);
            }
        },
        Value::Array(values) => {
            for child in values {
                assert_no_exact_keys(child, prohibited);
            }
        },
        _ => {},
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create(label: &str) -> Self {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "goodscanner-hsr-capture-test-{}-{label}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("test directory must be unique");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Ok(entries) = fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(&self.0);
    }
}
