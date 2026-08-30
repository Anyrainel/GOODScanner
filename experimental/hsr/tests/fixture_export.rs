use std::{fs, path::PathBuf};

use hsr_scanner_experimental::{
    build_export, parse_sanitized_fixture, FixtureObservationSource, JsonFileReferenceProvider,
    Language, ObservationSource, ReferenceCache,
};
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn observation_value() -> Value {
    serde_json::from_str(
        &fs::read_to_string(fixture("observations.json")).expect("fixture must exist"),
    )
    .expect("fixture must be JSON")
}

fn load_export() -> hsr_scanner_experimental::HsrInventoryExport {
    let cache = ReferenceCache::from_provider(&JsonFileReferenceProvider::new(fixture(
        "reference_cache.json",
    )))
    .expect("reference fixture must be valid");
    let observations = FixtureObservationSource::new(fixture("observations.json"))
        .load()
        .expect("observation fixture must be valid");
    build_export(observations, &cache).expect("fixture export must resolve")
}

#[test]
fn fixture_proves_all_four_inventory_categories() {
    let export = load_export();
    assert_eq!(export.characters.len(), 1);
    assert_eq!(export.light_cones.len(), 1);
    assert_eq!(export.relics.len(), 1);
    assert_eq!(export.planar_ornaments.len(), 1);
    assert!(!export.privacy.account_identifiers_included);
    assert!(!export.privacy.raw_packet_data_included);
}

#[test]
fn fixture_export_matches_the_golden_document() {
    let actual = serde_json::to_value(load_export()).expect("export must serialize");
    let expected: Value = serde_json::from_str(
        &fs::read_to_string(fixture("expected_export.json")).expect("golden fixture must exist"),
    )
    .expect("golden fixture must be JSON");
    assert_eq!(actual, expected);
}

#[test]
fn unknown_status_is_preserved_instead_of_inventing_false() {
    let actual = serde_json::to_value(load_export()).expect("export must serialize");
    assert_eq!(
        actual.pointer("/planarOrnaments/0/lock"),
        Some(&Value::Null)
    );
    assert_eq!(
        actual.pointer("/planarOrnaments/0/discard"),
        Some(&Value::Null)
    );
}

#[test]
fn prohibited_identity_field_is_rejected_before_typed_parsing() {
    for field in [
        "playerUid",
        "accessToken",
        "sessionId",
        "localId",
        "locationKey",
    ] {
        let mut value = observation_value();
        value
            .as_object_mut()
            .expect("fixture root must be an object")
            .insert(
                field.to_string(),
                Value::String("must-not-leak".to_string()),
            );
        let error = parse_sanitized_fixture(&value.to_string())
            .expect_err("identity/session field must be rejected");
        assert_eq!(error.code(), "HSR-DATA-SENSITIVE");
        assert!(!error
            .localized_message(Language::En)
            .contains("must-not-leak"));
    }

    let mut value = observation_value();
    value
        .as_object_mut()
        .expect("fixture root must be an object")
        .insert(
            "playerUid".to_string(),
            Value::String("redacted".to_string()),
        );
    let error =
        parse_sanitized_fixture(&value.to_string()).expect_err("account identity must be rejected");
    assert!(error
        .localized_message(Language::En)
        .starts_with("The input contains a prohibited account or session field"));
    assert!(error
        .localized_message(Language::ZhCn)
        .starts_with("输入包含禁止保留的账号或会话字段"));
    assert!(error
        .localized_message(Language::En)
        .contains("Full error details"));
    assert!(error
        .localized_message(Language::ZhCn)
        .contains("完整错误详情"));
}

#[test]
fn unresolved_reference_has_bilingual_readable_error_and_technical_chain() {
    let cache = ReferenceCache::from_provider(&JsonFileReferenceProvider::new(fixture(
        "reference_cache.json",
    )))
    .expect("reference fixture must be valid");
    let mut value = observation_value();
    value["characters"][0]["characterId"] = Value::from(42);
    let observations = parse_sanitized_fixture(&value.to_string())
        .expect("mutated semantic fixture must remain structurally valid");

    let error = build_export(observations, &cache).expect_err("unknown reference must fail");
    let english = error.localized_message(Language::En);
    let chinese = error.localized_message(Language::ZhCn);
    assert!(english.starts_with("An HSR observation could not be resolved"));
    assert!(english.contains("[HSR-REF-MISSING]"));
    assert!(!english.contains("42"));
    assert!(english.contains("observed identifier redacted"));
    assert!(chinese.starts_with("HSR 观测无法在当前参考数据中解析"));
    assert!(chinese.contains("[HSR-REF-MISSING]"));
}

#[test]
fn equipment_locations_are_resolved_to_canonical_character_keys() {
    let export = load_export();
    assert_eq!(
        export.light_cones[0].location_key.as_deref(),
        Some("FixtureNavigator")
    );
    assert_eq!(
        export.planar_ornaments[0].gear.location_key.as_deref(),
        Some("FixtureNavigator")
    );
}

#[test]
fn unknown_equipment_location_fails_without_accepting_an_arbitrary_key() {
    let cache = ReferenceCache::from_provider(&JsonFileReferenceProvider::new(fixture(
        "reference_cache.json",
    )))
    .expect("reference fixture must be valid");
    let mut value = observation_value();
    value["lightCones"][0]["equippedCharacterId"] = Value::from(42);
    let observations = parse_sanitized_fixture(&value.to_string())
        .expect("mutated semantic fixture must remain structurally valid");

    let error = build_export(observations, &cache).expect_err("unknown location must fail");
    let message = error.localized_message(Language::En);
    assert!(message.contains("kind=equippedCharacter"));
    assert!(!message.contains("42"));
    assert!(message.contains("observed identifier redacted"));
    assert!(message.contains("record=lightCones[0]"));
}

#[test]
fn committed_fixtures_pass_the_production_privacy_gates() {
    ReferenceCache::from_provider(&JsonFileReferenceProvider::new(fixture(
        "reference_cache.json",
    )))
    .expect("reference fixture must pass the shared privacy gate");
    FixtureObservationSource::new(fixture("observations.json"))
        .load()
        .expect("observation fixture must pass the shared privacy gate");
}
