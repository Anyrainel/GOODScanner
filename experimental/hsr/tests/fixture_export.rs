use std::{fs, path::PathBuf};

use hsr_scanner::reference::GiloreBundleReferenceProvider;
use hsr_scanner::{
    build_export, parse_sanitized_fixture, EvidenceKind, FixtureObservationSource,
    JsonFileReferenceProvider, Language, ObservationSnapshot, ObservationSource, ReferenceCache,
    ValidatedObservationSnapshot,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

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

fn load_export() -> hsr_scanner::HsrInventoryExport {
    let cache = ReferenceCache::from_provider(&GiloreBundleReferenceProvider::new(fixture(
        "gilore_bundle",
    )))
    .expect("GIlore bundle fixture must be valid");
    let observations = FixtureObservationSource::new(fixture("observations.json"))
        .load()
        .expect("observation fixture must be valid");
    build_export(observations, &cache).expect("fixture export must resolve")
}

fn load_screen_export() -> hsr_scanner::HsrInventoryExport {
    let cache = ReferenceCache::from_provider(&GiloreBundleReferenceProvider::new(fixture(
        "gilore_bundle",
    )))
    .expect("GIlore bundle fixture must be valid");
    let mut snapshot: ObservationSnapshot =
        serde_json::from_value(observation_value()).expect("observation fixture must be typed");
    snapshot.evidence.kind = EvidenceKind::ScreenCapture;
    snapshot.evidence.revision = "screen-capture-fixture-v2".to_string();
    let observations = ValidatedObservationSnapshot::from_screen_capture(snapshot)
        .expect("synthetic screen observation must pass production gates");
    build_export(observations, &cache).expect("screen fixture must resolve")
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
    assert!(!export.privacy.server_item_identifiers_included);
}

#[test]
fn inventory_v3_preserves_the_legacy_v2_payload() {
    let actual = serde_json::to_value(load_screen_export()).expect("export must serialize");
    let mut expected: Value = serde_json::from_slice(
        &fs::read(fixture("expected_export.json")).expect("legacy golden must exist"),
    )
    .expect("legacy golden must be JSON");
    expected["schema"] = Value::from("goodscanner.hsr");
    expected["schemaVersion"] = Value::from(3);
    assert_eq!(actual, expected);
}

#[test]
fn sanitized_fixture_and_screen_golden_differ_only_in_explicit_provenance() {
    let mut sanitized = serde_json::to_value(load_export()).expect("fixture export must serialize");
    let mut screen: Value = serde_json::from_str(
        &fs::read_to_string(fixture("expected_export.json")).expect("golden fixture must exist"),
    )
    .expect("screen golden must be JSON");
    screen["schema"] = Value::from("goodscanner.hsr");
    screen["schemaVersion"] = Value::from(3);

    assert_eq!(sanitized["source"]["kind"], "sanitizedFixture");
    assert_eq!(sanitized["source"]["revision"], "sanitized-gilore-v2");
    assert_eq!(screen["source"]["kind"], "screenCapture");
    assert_eq!(screen["source"]["revision"], "screen-capture-fixture-v2");

    sanitized["source"]["kind"] = screen["source"]["kind"].clone();
    sanitized["source"]["revision"] = screen["source"]["revision"].clone();
    assert_eq!(sanitized, screen);
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
fn v3_uses_canonical_property_keys_without_invented_numeric_stat_ids() {
    let actual = serde_json::to_value(load_export()).expect("export must serialize");
    assert_eq!(actual["schemaVersion"], Value::from(3));
    assert_eq!(
        actual.pointer("/relics/0/mainStat/key"),
        Some(&Value::from("HPDelta"))
    );
    assert_eq!(
        actual.pointer("/relics/0/substats/0/key"),
        Some(&Value::from("CriticalChanceBase"))
    );
    assert!(actual.pointer("/relics/0/mainStat/gameId").is_none());
    assert!(actual.pointer("/relics/0/substats/0/gameId").is_none());
}

#[test]
fn legacy_v2_goldens_remain_byte_exact() {
    for (name, expected_sha256) in [
        (
            "expected_export.json",
            "3734ff182b1e7d6511a00262457ed9e219a103c7f9a1e1baaf0f596d90a2d373",
        ),
        (
            "capture_expected_export.json",
            "24c5e8359c61599225ae23889bb087636dc3b818c095bf7f8abd4ac33f979dfb",
        ),
    ] {
        let bytes = fs::read(fixture(name)).expect("legacy fixture must exist");
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), expected_sha256);
        let value: Value = serde_json::from_slice(&bytes).expect("legacy fixture must be JSON");
        assert_eq!(value["schema"], "goodscanner.hsr.experimental");
        assert_eq!(value["schemaVersion"], 2);
        assert!(value.get("achievements").is_none());
    }
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
    assert_eq!(export.light_cones[0].location_key.as_deref(), Some("1001"));
    assert_eq!(
        export.planar_ornaments[0].gear.location_key.as_deref(),
        Some("1001")
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

#[test]
fn observation_semantics_match_the_website_v3_numeric_contract() {
    for (pointer, invalid) in [
        ("/characters/0/level", Value::from(0)),
        ("/characters/0/ascension", Value::from(9)),
        ("/characters/0/eidolon", Value::from(7)),
        ("/lightCones/0/level", Value::from(101)),
        ("/lightCones/0/superimposition", Value::from(0)),
        ("/gear/0/level", Value::from(16)),
    ] {
        let mut value = observation_value();
        *value
            .pointer_mut(pointer)
            .expect("fixture pointer must resolve") = invalid;
        let error = parse_sanitized_fixture(&value.to_string())
            .expect_err("out-of-contract game values must fail before export");
        assert_eq!(error.code(), "HSR-OBS-INVALID", "pointer={pointer}");
    }

    let mut duplicate = observation_value();
    let duplicate_character = duplicate["characters"][0].clone();
    duplicate["characters"]
        .as_array_mut()
        .expect("characters must be an array")
        .push(duplicate_character);
    let error = parse_sanitized_fixture(&duplicate.to_string())
        .expect_err("duplicate character definitions must fail before export");
    assert_eq!(error.code(), "HSR-OBS-INVALID");
}
