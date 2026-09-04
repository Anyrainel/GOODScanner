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
        "7ef3650a63622c204b89234406c99dc221e01d85"
    );
    assert_eq!(
        EMBEDDED_GILORE_SOURCE_REVISION,
        "8cdb905dc2f8e6fffa9be4eb07af3e34435d6091"
    );
    assert_eq!(
        EMBEDDED_GILORE_MANIFEST_SHA256,
        "9899cc8fdde578cdbd744ec9f8b2705cd2f11d43670232e871f489fc3d549b5f"
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
