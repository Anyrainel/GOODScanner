use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use hsr_scanner::{
    model::{
        AchievementCoverage, AchievementSource, AchievementSourceKind, AchievementStatus,
        EvidenceKind, HsrAchievementEntry, HsrAchievementSnapshot,
    },
    pipeline::{
        build_achievement_only_export, build_achievement_snapshot, build_export,
        build_export_with_achievements, write_export_create_new,
    },
    FixtureObservationSource, GiloreBundleReferenceProvider, ObservationSource, ReferenceCache,
    ReferenceProvider,
};
use serde_json::Value;

// Historical v3 export fixture remains readable after decoder replacement.
const CAPTURE_REVISION: &str = "auto-reliquary-1.2.0";

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn references() -> ReferenceCache {
    let provider = GiloreBundleReferenceProvider::new(fixture("gilore_bundle"));
    let mut snapshot = provider.load().expect("reference fixture must load");
    snapshot.achievement_ids = vec![4_040_201, 4_010_101, 4_010_102];
    ReferenceCache::from_snapshot(snapshot).expect("achievement reference must validate")
}

fn inventory_only_references() -> ReferenceCache {
    ReferenceCache::from_provider(&GiloreBundleReferenceProvider::new(fixture(
        "gilore_bundle",
    )))
    .expect("legacy inventory reference must load")
}

fn observations() -> hsr_scanner::ValidatedObservationSnapshot {
    FixtureObservationSource::new(fixture("observations.json"))
        .load()
        .expect("observation fixture must load")
}

#[test]
fn inventory_only_v3_omits_unobserved_achievements() {
    let export = build_export(observations(), &inventory_only_references())
        .expect("inventory-only v3 export must build");
    let value = serde_json::to_value(export).expect("export must serialize");

    assert_eq!(value["schema"], "goodscanner.hsr");
    assert_eq!(value["schemaVersion"], 3);
    assert!(value.get("achievements").is_none());
}

#[test]
fn completed_ids_are_reference_validated_sorted_and_deduplicated() {
    let references = references();
    let snapshot = build_achievement_snapshot(
        [4_040_201, 4_010_102, 4_010_101, 4_040_201],
        CAPTURE_REVISION,
        &references,
    )
    .expect("known IDs must normalize");

    assert_eq!(snapshot.source.kind, AchievementSourceKind::PacketCapture);
    assert_eq!(snapshot.coverage, AchievementCoverage::Complete);
    assert_eq!(
        snapshot
            .entries
            .iter()
            .map(|entry| entry.achievement_id)
            .collect::<Vec<_>>(),
        vec![4_010_101, 4_010_102, 4_040_201]
    );
    assert!(snapshot
        .entries
        .iter()
        .all(|entry| entry.status == AchievementStatus::Completed));
}

#[test]
fn mixed_export_keeps_inventory_and_achievement_provenance_separate() {
    let references = references();
    let achievements =
        build_achievement_snapshot([4_010_101], CAPTURE_REVISION, &references).unwrap();
    let export = build_export_with_achievements(observations(), achievements, &references)
        .expect("mixed export must build");

    assert_eq!(export.source.kind, EvidenceKind::SanitizedFixture);
    assert_eq!(export.source.revision, "sanitized-gilore-v2");
    assert_eq!(export.characters.len(), 1);
    let achievements = export.achievements.expect("snapshot must be attached");
    assert_eq!(
        achievements.source.kind,
        AchievementSourceKind::PacketCapture
    );
    assert_eq!(achievements.source.revision, CAPTURE_REVISION);
}

#[test]
fn achievement_only_export_matches_v3_golden() {
    let references = references();
    let achievements = build_achievement_snapshot(
        [4_040_201, 4_010_101, 4_040_201],
        CAPTURE_REVISION,
        &references,
    )
    .unwrap();
    let export = build_achievement_only_export(achievements, &references).unwrap();
    let actual = format!(
        "{}\n",
        serde_json::to_string_pretty(&export).expect("export must serialize")
    );
    let expected = fs::read_to_string(fixture("achievement_only_export_v3.json"))
        .expect("v3 golden must exist");

    assert_eq!(actual.as_bytes(), expected.as_bytes());
}

#[test]
fn complete_empty_snapshot_is_present_and_authoritative() {
    let references = references();
    let achievements =
        build_achievement_snapshot([], CAPTURE_REVISION, &references).expect("empty is valid");
    let export = build_achievement_only_export(achievements, &references).unwrap();
    let value = serde_json::to_value(export).unwrap();

    assert_eq!(value["achievements"]["coverage"], "complete");
    assert_eq!(value["achievements"]["entries"], Value::Array(Vec::new()));
    assert_eq!(value["source"]["coverage"]["characters"], "unknown");
    assert_eq!(value["source"]["coverage"]["lightCones"], "unknown");
    assert_eq!(value["source"]["coverage"]["relics"], "unknown");
}

#[test]
fn malformed_or_unknown_snapshots_fail_before_export() {
    let references = references();
    for (label, snapshot) in [
        ("unsafe revision", snapshot("account=secret", &[4_010_101])),
        (
            "unsorted",
            snapshot(CAPTURE_REVISION, &[4_040_201, 4_010_101]),
        ),
        (
            "duplicate",
            snapshot(CAPTURE_REVISION, &[4_010_101, 4_010_101]),
        ),
        ("zero", snapshot(CAPTURE_REVISION, &[0])),
        ("unknown", snapshot(CAPTURE_REVISION, &[4_999_999])),
    ] {
        let error = build_achievement_only_export(snapshot, &references)
            .expect_err("malformed snapshot must fail");
        assert_eq!(error.code(), "HSR-ACHIEVEMENT-INVALID", "case={label}");
    }

    let error =
        build_achievement_snapshot([4_010_101], CAPTURE_REVISION, &inventory_only_references())
            .expect_err("v1.1 inventory reference cannot authorize achievements");
    assert_eq!(error.code(), "HSR-ACHIEVEMENT-INVALID");
}

#[test]
fn achievement_wire_literals_and_fields_are_strict() {
    let references = references();
    let valid = build_achievement_snapshot([4_010_101], CAPTURE_REVISION, &references).unwrap();
    for (pointer, replacement) in [
        ("/source/kind", Value::from("screenCapture")),
        ("/coverage", Value::from("unknown")),
        ("/entries/0/status", Value::from("inProgress")),
    ] {
        let mut value = serde_json::to_value(&valid).unwrap();
        *value.pointer_mut(pointer).expect("test pointer must exist") = replacement;
        serde_json::from_value::<HsrAchievementSnapshot>(value)
            .expect_err("unsupported literals must fail strict deserialization");
    }

    let mut unknown = serde_json::to_value(valid).unwrap();
    unknown["unexpected"] = Value::Bool(true);
    serde_json::from_value::<HsrAchievementSnapshot>(unknown)
        .expect_err("unknown snapshot fields must fail strict deserialization");
}

#[test]
fn writer_creates_once_without_overwriting() {
    let directory = TestDirectory::create();
    let path = directory.path().join("hsr-export.json");
    let references = references();
    let achievements =
        build_achievement_snapshot([4_010_101], CAPTURE_REVISION, &references).unwrap();
    let export = build_achievement_only_export(achievements, &references).unwrap();

    write_export_create_new(&path, &export).expect("first write must succeed");
    let original = fs::read(&path).expect("created export must be readable");
    assert_eq!(original.last(), Some(&b'\n'));
    let error = write_export_create_new(&path, &export).expect_err("overwrite must be rejected");
    assert_eq!(error.code(), "HSR-EXPORT-CREATE");
    assert_eq!(fs::read(&path).unwrap(), original);
}

fn snapshot(revision: &str, ids: &[u32]) -> HsrAchievementSnapshot {
    HsrAchievementSnapshot {
        source: AchievementSource {
            kind: AchievementSourceKind::PacketCapture,
            revision: revision.to_string(),
        },
        coverage: AchievementCoverage::Complete,
        entries: ids
            .iter()
            .copied()
            .map(|achievement_id| HsrAchievementEntry {
                achievement_id,
                status: AchievementStatus::Completed,
            })
            .collect(),
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "goodscanner-hsr-export-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("unique test directory must be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
