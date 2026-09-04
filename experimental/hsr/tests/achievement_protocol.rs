#![cfg(feature = "capture")]

use std::collections::BTreeSet;

use base64::prelude::*;
use hsr_scanner::achievement_capture::{
    protocol::decode_achievement_command, ACHIEVEMENT_CAPTURE_REVISION,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProtocolFixture {
    fixture_kind: String,
    description: String,
    known_achievement_ids: Vec<u32>,
    cases: Vec<ProtocolCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProtocolCase {
    name: String,
    proto_base64: String,
    #[serde(default)]
    expected_completed_ids: Option<Vec<u32>>,
    #[serde(default)]
    expected_error_code: Option<String>,
    #[serde(default)]
    expected_absent: bool,
}

fn fixture() -> ProtocolFixture {
    serde_json::from_str(include_str!("fixtures/achievement_protocol.json"))
        .expect("synthetic achievement protocol fixture must parse")
}

#[test]
fn synthetic_fixture_is_explicitly_privacy_safe() {
    let fixture = fixture();
    assert_eq!(fixture.fixture_kind, "syntheticDecryptedCommand");
    assert!(fixture.description.contains("no account identifiers"));
    assert!(fixture.description.contains("no raw captured packets"));
    assert_eq!(ACHIEVEMENT_CAPTURE_REVISION, "auto-reliquary-1.2.0");
}

#[test]
fn replay_normalizes_only_completed_statuses_and_rotated_fields() {
    let fixture = fixture();
    let known_ids = fixture
        .known_achievement_ids
        .into_iter()
        .collect::<BTreeSet<_>>();

    for case in fixture.cases {
        let Some(expected_ids) = case.expected_completed_ids else {
            continue;
        };
        let proto = BASE64_STANDARD
            .decode(&case.proto_base64)
            .unwrap_or_else(|error| panic!("{} fixture base64 invalid: {error}", case.name));
        let decoded = decode_achievement_command(&proto, &known_ids)
            .unwrap_or_else(|error| panic!("{} unexpectedly failed: {error}", case.name))
            .unwrap_or_else(|| panic!("{} was not recognized", case.name));
        assert_eq!(decoded.completed_ids(), expected_ids, "{}", case.name);
    }
}

#[test]
fn replay_distinguishes_absence_from_a_present_empty_snapshot() {
    let fixture = fixture();
    let known_ids = fixture
        .known_achievement_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    let empty = fixture
        .cases
        .iter()
        .find(|case| case.name == "present-complete-empty")
        .unwrap();
    let proto = BASE64_STANDARD.decode(&empty.proto_base64).unwrap();
    let decoded = decode_achievement_command(&proto, &known_ids)
        .unwrap()
        .expect("a complete response with zero completed achievements is present");
    assert!(decoded.completed_ids().is_empty());

    for case in fixture.cases.iter().filter(|case| case.expected_absent) {
        let proto = BASE64_STANDARD.decode(&case.proto_base64).unwrap();
        assert_eq!(
            decode_achievement_command(&proto, &known_ids).unwrap(),
            None,
            "{}",
            case.name
        );
    }
}

#[test]
fn replay_rejects_unknown_public_ids_instead_of_silently_dropping_them() {
    let fixture = fixture();
    let known_ids = fixture
        .known_achievement_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let case = fixture
        .cases
        .iter()
        .find(|case| case.expected_error_code.is_some())
        .unwrap();
    let proto = BASE64_STANDARD.decode(&case.proto_base64).unwrap();
    let error = decode_achievement_command(&proto, &known_ids).unwrap_err();

    assert_eq!(error.code(), case.expected_error_code.as_deref().unwrap());
    assert!(error.to_string().contains("4999999"));
}
