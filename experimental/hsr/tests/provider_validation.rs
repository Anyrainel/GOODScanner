use std::{fs, path::PathBuf};

use hsr_scanner_experimental::{
    GearCategory, GearSlot, Language, ReferenceCache, ReferenceSnapshot,
};

fn reference_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("reference_cache.json")
}

fn snapshot() -> ReferenceSnapshot {
    serde_json::from_str(
        &fs::read_to_string(reference_fixture()).expect("reference fixture must exist"),
    )
    .expect("reference fixture must be valid")
}

#[test]
fn provider_rejects_duplicate_semantic_ids() {
    let mut snapshot = snapshot();
    snapshot.characters.push(snapshot.characters[0].clone());
    let error = ReferenceCache::from_snapshot(snapshot).expect_err("duplicate ID must fail");
    assert_eq!(error.code(), "HSR-REF-INVALID");
    assert!(error
        .localized_message(Language::En)
        .contains("duplicate gameId=1001"));
}

#[test]
fn provider_rejects_duplicate_stable_keys() {
    let mut snapshot = snapshot();
    let mut duplicate = snapshot.characters[0].clone();
    duplicate.game_id += 1;
    snapshot.characters.push(duplicate);
    let error = ReferenceCache::from_snapshot(snapshot).expect_err("duplicate key must fail");
    assert_eq!(error.code(), "HSR-REF-INVALID");
    assert!(error
        .localized_message(Language::En)
        .contains("duplicate key=1001 in characters"));
}

#[test]
fn provider_owns_planar_classification_and_validates_slot_semantics() {
    let mut snapshot = snapshot();
    snapshot.gear_pieces[0].category = GearCategory::PlanarOrnament;
    snapshot.gear_pieces[0].slot = GearSlot::Head;
    let error = ReferenceCache::from_snapshot(snapshot)
        .expect_err("category/slot mismatch must fail closed");
    assert!(error
        .localized_message(Language::En)
        .contains("category does not match slot Head"));
    assert!(error
        .localized_message(Language::ZhCn)
        .starts_with("HSR 参考数据缓存无效"));
}
