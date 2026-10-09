use good_tools_app::{
    config::StarRailSettings,
    gui::{star_rail_worker::finish_scan_export, state::TaskStatus},
};
use hsr_scanner::{
    model::{
        CoverageLevel, EvidenceKind, InventoryCoverage, ObservationSnapshot, ReferenceSnapshot,
    },
    observation::{parse_sanitized_fixture, ValidatedObservationSnapshot},
    reference::ReferenceCache,
    scanner::ScanResult,
    CaptureExportDetails,
};
use std::{fs, path::PathBuf};

fn temp_dir() -> PathBuf {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "hsr-scan-export-test-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}

fn remove_temp_dir(dir: &std::path::Path) {
    let resolved = dir.canonicalize().unwrap();
    assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    assert!(resolved
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("hsr-scan-export-test-"));
    fs::remove_dir_all(resolved).unwrap();
}

fn result(empty: bool, complete: bool) -> (ScanResult, ReferenceCache) {
    let observations = if empty {
        let mut snapshot: ObservationSnapshot = serde_json::from_str(include_str!(
            "../../experimental/hsr/tests/fixtures/observations.json"
        ))
        .unwrap();
        snapshot.evidence.kind = EvidenceKind::ScreenCapture;
        snapshot.characters.clear();
        snapshot.light_cones.clear();
        snapshot.gear.clear();
        ValidatedObservationSnapshot::from_screen_capture(snapshot).unwrap()
    } else {
        parse_sanitized_fixture(include_str!(
            "../../experimental/hsr/tests/fixtures/observations.json"
        ))
        .unwrap()
    };
    let level = if complete {
        CoverageLevel::Complete
    } else {
        CoverageLevel::Unknown
    };
    let result = ScanResult {
        observations,
        gear_items: Vec::new(),
        coverage: InventoryCoverage {
            characters: level,
            light_cones: level,
            relics: level,
        },
        export_details: CaptureExportDetails::default(),
    };
    let reference: ReferenceSnapshot = serde_json::from_str(include_str!(
        "../../experimental/hsr/tests/fixtures/reference_cache.json"
    ))
    .unwrap();
    (result, ReferenceCache::from_snapshot(reference).unwrap())
}

#[test]
fn stopping_obeys_the_save_choice_and_never_writes_an_empty_export() {
    let dir = temp_dir();
    let mut settings = StarRailSettings {
        output_dir: dir.display().to_string(),
        ..Default::default()
    };
    let (scanned, refs) = result(false, false);
    assert!(matches!(
        finish_scan_export(&settings, &scanned, &refs, true, None).unwrap(),
        TaskStatus::Stopped(_)
    ));
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    settings.scan_save_on_cancel = true;
    let (empty, _) = result(true, false);
    assert!(matches!(
        finish_scan_export(&settings, &empty, &refs, true, None).unwrap(),
        TaskStatus::Stopped(_)
    ));
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    let TaskStatus::Exported { path, partial, .. } =
        finish_scan_export(&settings, &scanned, &refs, true, None).unwrap()
    else {
        panic!("stopped scan should save partial results")
    };
    assert!(partial);
    let json: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(json["version"], 4);
    assert_eq!(json["characters"].as_array().unwrap().len(), 1);
    remove_temp_dir(&dir);
}

#[test]
fn latest_scan_cleanup_preserves_capture_other_games_manual_and_future_files() {
    let dir = temp_dir();
    let old = "star_rail_scan_2026-01-01_00-00-00.json";
    let keep = [
        "star_rail_capture_2026-01-01_00-00-00.json",
        "genshin_export_2026-01-01_00-00-00.json",
        "good_export_2026-01-01_00-00-00.json",
        "star_rail_scan_latest.json",
        "star_rail_scan_2026-02-30_00-00-00.json",
        "star_rail_scan_9999-01-01_00-00-00.json",
    ];
    for name in keep.iter().chain(std::iter::once(&old)) {
        fs::write(dir.join(name), b"keep").unwrap();
    }
    let settings = StarRailSettings {
        output_dir: dir.display().to_string(),
        ..Default::default()
    };
    let (scanned, refs) = result(false, true);
    let TaskStatus::Exported { path, partial, .. } =
        finish_scan_export(&settings, &scanned, &refs, false, None).unwrap()
    else {
        panic!("full scan should save")
    };
    assert!(!partial);
    assert!(std::path::Path::new(&path).is_file());
    assert!(!dir.join(old).exists());
    for name in keep {
        assert_eq!(fs::read(dir.join(name)).unwrap(), b"keep");
    }
    remove_temp_dir(&dir);
}

#[test]
fn history_mode_keeps_both_exports_and_scan_caps_still_save_partial_results() {
    let dir = temp_dir();
    let settings = StarRailSettings {
        output_dir: dir.display().to_string(),
        scan_only_keep_latest_export: false,
        ..Default::default()
    };
    let (scanned, refs) = result(false, false);
    let mut paths = Vec::new();
    for _ in 0..2 {
        let TaskStatus::Exported { path, partial, .. } =
            finish_scan_export(&settings, &scanned, &refs, false, None).unwrap()
        else {
            panic!("capped scan should save independently of save-on-stop")
        };
        assert!(partial);
        paths.push(path);
    }
    assert_ne!(paths[0], paths[1]);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
    remove_temp_dir(&dir);
}

#[test]
fn a_write_failure_after_stop_is_reported_and_keeps_previous_results() {
    let dir = temp_dir();
    let old = dir.join("star_rail_scan_2026-01-01_00-00-00.json");
    fs::write(&old, b"previous complete export").unwrap();
    // A file cannot serve as the output directory, so finalization must fail
    // before cleanup, even when this was a user-cancelled partial scan.
    let settings = StarRailSettings {
        output_dir: old.display().to_string(),
        scan_save_on_cancel: true,
        ..Default::default()
    };
    let (scanned, refs) = result(false, false);
    let error = finish_scan_export(&settings, &scanned, &refs, true, None).unwrap_err();
    assert!(error
        .hint_text(good_tools_app::gui::state::Lang::En)
        .contains("folder"));
    assert_eq!(fs::read(old).unwrap(), b"previous complete export");
    remove_temp_dir(&dir);
}

#[test]
fn an_empty_scan_reports_the_problem_without_deleting_previous_results() {
    let dir = temp_dir();
    let old = dir.join("star_rail_scan_2026-01-01_00-00-00.json");
    fs::write(&old, b"previous complete export").unwrap();
    let settings = StarRailSettings {
        output_dir: dir.display().to_string(),
        ..Default::default()
    };
    let (empty, refs) = result(true, false);
    assert!(finish_scan_export(&settings, &empty, &refs, false, None)
        .unwrap_err()
        .hint_text(good_tools_app::gui::state::Lang::En)
        .contains("Nothing was exported"));
    assert_eq!(fs::read(old).unwrap(), b"previous complete export");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    remove_temp_dir(&dir);
}
