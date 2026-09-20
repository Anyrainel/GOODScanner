use hsr_scanner::{
    load_embedded_gilore_reference, EMBEDDED_GILORE_COMMIT, EMBEDDED_GILORE_MANIFEST_SHA256,
    EMBEDDED_GILORE_SOURCE_REVISION, EMBEDDED_REFERENCE_ACHIEVEMENT_COUNT,
    EMBEDDED_REFERENCE_BYTE_COUNT, EMBEDDED_REFERENCE_PROVIDER,
    EMBEDDED_REFERENCE_SENTINEL_ACHIEVEMENT_ID, EMBEDDED_REFERENCE_SHA256,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

const EMBEDDED_BYTES: &[u8] = include_bytes!("../assets/gilore_reference_v1.json");

#[test]
fn embedded_reference_is_integrity_pinned_private_and_live_complete() {
    assert_eq!(
        EMBEDDED_GILORE_COMMIT,
        "1ca018ee615011e61f90f11c3f47738e6ebac30b"
    );
    assert_eq!(
        EMBEDDED_GILORE_SOURCE_REVISION,
        "8cdb905dc2f8e6fffa9be4eb07af3e34435d6091"
    );
    assert_eq!(
        EMBEDDED_GILORE_MANIFEST_SHA256,
        "5acf3567d23a9a13531a62c448d075d218e824996ca25e994cef1c322bb89eb1"
    );
    assert_eq!(EMBEDDED_BYTES.len(), EMBEDDED_REFERENCE_BYTE_COUNT);
    assert_eq!(
        format!("{:x}", Sha256::digest(EMBEDDED_BYTES)),
        EMBEDDED_REFERENCE_SHA256
    );

    let value: Value = serde_json::from_slice(EMBEDDED_BYTES).expect("embedded JSON must parse");
    assert_eq!(
        serde_json::to_vec(&value).expect("embedded JSON must reserialize"),
        EMBEDDED_BYTES,
        "the checked-in document must remain deterministic minified JSON"
    );

    let cache = load_embedded_gilore_reference().expect("embedded reference must validate");
    cache
        .validate_live_complete_profile()
        .expect("embedded reference must be safe for account-facing flows");
    assert_eq!(cache.provider(), EMBEDDED_REFERENCE_PROVIDER);
    assert_eq!(cache.revision(), EMBEDDED_GILORE_SOURCE_REVISION);
    assert_eq!(
        cache.achievement_count(),
        EMBEDDED_REFERENCE_ACHIEVEMENT_COUNT
    );
    assert!(cache.has_achievement(EMBEDDED_REFERENCE_SENTINEL_ACHIEVEMENT_ID));
}
