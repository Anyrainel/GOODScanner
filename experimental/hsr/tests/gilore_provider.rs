use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use hsr_scanner_experimental::reference::{GiloreBundleReferenceProvider, ReferenceProvider};
use hsr_scanner_experimental::{
    build_export, parse_sanitized_fixture, EvidenceKind, FixtureObservationSource, GearCategory,
    GearSlot, ObservationSource, ReferenceCache, StatValueKind,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

const GILORE_REVISION: &str = "014e33e2404f8cd668bf06fc2ea6db53b6bc3992";
const BUNDLE_FILES: [&str; 7] = [
    "manifest.json",
    "characters.json",
    "light_cones.json",
    "relic_sets.json",
    "relic_pieces.json",
    "property_tables.json",
    "progression.json",
];

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn load_cache(root: impl AsRef<Path>) -> ReferenceCache {
    ReferenceCache::from_provider(&GiloreBundleReferenceProvider::new(root.as_ref()))
        .expect("sanitized GIlore bundle must load")
}

#[test]
fn gilore_bundle_resolves_audited_public_ids_and_canonical_property_keys() {
    let cache = load_cache(fixture("gilore_bundle"));

    assert_eq!(cache.provider(), "gilore.ggstarrail-reference");
    assert_eq!(cache.revision(), GILORE_REVISION);
    assert_eq!(cache.character(1001).expect("March 7th").key, "1001");
    assert_eq!(
        cache.light_cone(23005).expect("Moment of Victory").name.en,
        "Moment of Victory"
    );

    let relic = cache.gear(61011).expect("cavern relic");
    assert_eq!(relic.category, GearCategory::Relic);
    assert_eq!(relic.slot, GearSlot::Head);
    assert_eq!(relic.set_key, "101");

    let planar = cache.gear(63015).expect("planar ornament");
    assert_eq!(planar.category, GearCategory::PlanarOrnament);
    assert_eq!(planar.slot, GearSlot::PlanarSphere);
    assert_eq!(planar.set_key, "301");

    assert_eq!(
        cache.relic_main_stat_value("101", GearSlot::Head, 5, "HPDelta", 15),
        Some(705.6)
    );
    assert_eq!(
        cache.relic_main_stat_value("301", GearSlot::PlanarSphere, 5, "AttackAddedRatio", 15),
        Some(43.2)
    );
    assert_eq!(
        cache.relic_main_stat_value("101", GearSlot::Head, 5, "HPDelta", 16),
        None,
        "level drift must fail closed"
    );

    assert_eq!(
        cache.stat("HPDelta").expect("flat HP property").name.zh_cn,
        "生命值"
    );
    assert_eq!(
        cache
            .stat("CriticalChanceBase")
            .expect("critical chance property")
            .name
            .en,
        "CRIT Rate"
    );
}

#[test]
fn duplicate_four_star_buckets_use_main_affix_then_visible_equivalence() {
    let cache = load_cache(fixture("gilore_bundle"));
    let candidates = cache.gear_candidates_by_set_slot_rarity("101", GearSlot::Body, 4);
    assert_eq!(
        candidates
            .iter()
            .map(|entry| entry.game_id)
            .collect::<Vec<_>>(),
        vec![51013, 55001]
    );
    assert_eq!(
        cache.gear_by_set_slot_rarity("101", GearSlot::Body, 4),
        None,
        "set/slot/rarity alone must not select an arbitrary public definition"
    );

    let attack = cache
        .resolve_gear_by_set_slot_rarity_main_stat("101", GearSlot::Body, 4, "AttackAddedRatio", 12)
        .expect("ATK exists only in Passerby group 43");
    assert_eq!(attack.game_id, 51013);
    let attack_value = cache
        .relic_main_stat_value_for_piece(attack, "AttackAddedRatio", 12)
        .expect("authoritative ATK progression value");
    assert!((attack_value - 28.7544).abs() < 1e-9);

    let healing = cache
        .resolve_gear_by_set_slot_rarity_main_stat("101", GearSlot::Body, 4, "HealRatioBase", 12)
        .expect("identical visible/progression definitions have a canonical identity");
    assert_eq!(healing.game_id, 51013, "lowest public numeric ID wins");
    assert_eq!(
        cache
            .canonical_gear_for_observation(55001, "HealRatioBase", 12, 23.0)
            .map(|entry| entry.game_id),
        Some(51013),
        "a supplied alternate ID selects the class, not the canonical member"
    );
    assert_eq!(
        cache.canonical_gear_for_observation(55001, "AttackAddedRatio", 12, 28.7),
        None,
        "a supplied ID cannot claim a property absent from its main-affix group"
    );

    assert_eq!(
        cache
            .resolve_gear_name_with_main_stat("过客的残绣风衣", 4, "AttackAddedRatio", 12, 28.7,)
            .map(|entry| entry.game_id),
        Some(51013),
        "captured one-decimal display values validate against exact progression"
    );
    assert_eq!(
        cache.resolve_gear_name_with_main_stat(
            "Passerby's Ragged Embroidered Coat",
            4,
            "AttackAddedRatio",
            12,
            99.9,
        ),
        None,
        "a stale or impossible visible value must fail closed"
    );
}

#[test]
fn export_canonicalizes_a_supplied_equivalent_public_piece_id() {
    let cache = load_cache(fixture("gilore_bundle"));
    let mut value: Value = serde_json::from_slice(
        &fs::read(fixture("observations.json")).expect("observation fixture"),
    )
    .expect("observation JSON");
    let gear = value["gear"].as_array_mut().expect("gear array");
    gear.truncate(1);
    gear[0]["pieceId"] = Value::from(55001);
    gear[0]["level"] = Value::from(12);
    gear[0]["mainStatKey"] = Value::from("HealRatioBase");
    gear[0]["mainStatValue"] = Value::from(23.0033);

    let observations = parse_sanitized_fixture(&value.to_string()).expect("valid alternate ID");
    let export = build_export(observations, &cache).expect("canonical export");
    assert_eq!(export.relics.len(), 1);
    assert_eq!(export.relics[0].gear.game_id, 51013);
    assert_eq!(export.relics[0].gear.key, "51013");
}

#[test]
fn visible_equivalence_requires_icon_and_exact_full_progression() {
    let provider = GiloreBundleReferenceProvider::new(fixture("gilore_bundle"));

    let mut icon_drift = provider.load().expect("fixture snapshot");
    icon_drift
        .gear_pieces
        .iter_mut()
        .find(|entry| entry.game_id == 55001)
        .expect("alternate Passerby definition")
        .icon_path = "SpriteOutput/ItemIcon/RelicIcons/Unexpected.png".to_string();
    let icon_drift = ReferenceCache::from_snapshot(icon_drift).expect("valid drift fixture");
    assert_eq!(
        icon_drift.resolve_gear_by_set_slot_rarity_main_stat(
            "101",
            GearSlot::Body,
            4,
            "HealRatioBase",
            12,
        ),
        None
    );

    let mut progression_drift = provider.load().expect("fixture snapshot");
    progression_drift
        .relic_main_affixes
        .iter_mut()
        .find(|entry| entry.group_id == 436 && entry.property_id == "HealRatioBase")
        .expect("alternate healing progression")
        .level_values[0] += 0.000001;
    let progression_drift =
        ReferenceCache::from_snapshot(progression_drift).expect("valid drift fixture");
    assert_eq!(
        progression_drift.resolve_gear_by_set_slot_rarity_main_stat(
            "101",
            GearSlot::Body,
            4,
            "HealRatioBase",
            12,
        ),
        None
    );
}

#[test]
fn colliding_stat_labels_require_flat_or_ratio_evidence() {
    let cache = load_cache(fixture("gilore_bundle"));
    assert_eq!(cache.resolve_stat_name("HP"), None);
    assert_eq!(
        cache
            .resolve_stat_name_by_kind("HP", StatValueKind::Flat)
            .map(|entry| entry.key.as_str()),
        Some("HPDelta")
    );
    assert_eq!(
        cache
            .resolve_stat_name_by_kind("生命值", StatValueKind::Ratio)
            .map(|entry| entry.key.as_str()),
        Some("HPAddedRatio")
    );
    assert_eq!(cache.resolve_stat_name("ATK"), None);
    assert_eq!(
        cache
            .resolve_stat_name_by_kind("攻击力", StatValueKind::Flat)
            .map(|entry| entry.key.as_str()),
        Some("AttackDelta")
    );
    assert_eq!(
        cache
            .resolve_stat_name_by_kind("ATK", StatValueKind::Ratio)
            .map(|entry| entry.key.as_str()),
        Some("AttackAddedRatio")
    );
}

#[test]
fn duplicate_character_names_use_visible_path_but_not_hidden_sex() {
    let provider = GiloreBundleReferenceProvider::new(fixture("gilore_bundle"));
    let mut snapshot = provider.load().expect("fixture snapshot");
    let mut hunt_march = snapshot.characters[0].clone();
    hunt_march.game_id = 1224;
    hunt_march.key = "1224".to_string();
    hunt_march.path = "Rogue".to_string();
    snapshot.characters.push(hunt_march);

    let mut stelle = snapshot.characters[0].clone();
    stelle.game_id = 8001;
    stelle.key = "8001".to_string();
    stelle.name.zh_cn = "{NICKNAME}".to_string();
    stelle.name.en = "{NICKNAME}".to_string();
    stelle.path = "Warrior".to_string();
    let mut caelus = stelle.clone();
    caelus.game_id = 8002;
    caelus.key = "8002".to_string();
    snapshot.characters.extend([stelle, caelus]);

    let cache = ReferenceCache::from_snapshot(snapshot).expect("variant fixture");
    assert_eq!(cache.resolve_character_name("March 7th"), None);
    assert_eq!(
        cache
            .resolve_character_name_and_path("三月七", "存护")
            .map(|entry| entry.game_id),
        Some(1001)
    );
    assert_eq!(
        cache
            .resolve_character_name_and_path("March 7th", "The Hunt")
            .map(|entry| entry.game_id),
        Some(1224)
    );
    assert_eq!(
        cache.resolve_character_name_and_path("{NICKNAME}", "Destruction"),
        None,
        "visible path cannot distinguish Trailblazer sex definitions"
    );
}

#[test]
fn export_reports_gilore_provenance_and_sanitized_fixture_evidence_separately() {
    let cache = load_cache(fixture("gilore_bundle"));
    let observations = FixtureObservationSource::new(fixture("observations.json"))
        .load()
        .expect("observation fixture must load");
    let export = build_export(observations, &cache).expect("fixture must resolve through GIlore");

    assert_eq!(export.source.kind, EvidenceKind::SanitizedFixture);
    assert_eq!(export.source.revision, "sanitized-gilore-v2");
    assert_eq!(export.reference.provider, "gilore.ggstarrail-reference");
    assert_eq!(export.reference.revision, GILORE_REVISION);
    assert_eq!(export.characters[0].game_id, 1001);
    assert_eq!(export.light_cones[0].game_id, 23005);
    assert_eq!(export.relics[0].gear.main_stat.key, "HPDelta");
}

#[test]
fn provider_rejects_a_member_whose_bytes_do_not_match_the_manifest_hash() {
    let bundle = CopiedBundle::new("bad-hash");
    let member_path = bundle.path().join("characters.json");
    fs::OpenOptions::new()
        .append(true)
        .open(&member_path)
        .expect("copied character member must open")
        .write_all(b" ")
        .expect("test must alter copied member");
    update_manifest_file_metadata(
        bundle.path(),
        "characters.json",
        Some(fs::metadata(&member_path).unwrap().len()),
        None,
        None,
    );

    let error = ReferenceCache::from_provider(&GiloreBundleReferenceProvider::new(bundle.path()))
        .expect_err("hash mismatch must fail closed");
    assert_eq!(error.code(), "HSR-GILORE-HASH");
    assert!(error
        .localized_message(hsr_scanner_experimental::Language::En)
        .contains("characters.json"));
}

#[test]
fn provider_rejects_cross_revision_members_even_when_their_hash_is_valid() {
    let bundle = CopiedBundle::new("mixed-revision");
    let member_path = bundle.path().join("characters.json");
    let mut member: Value =
        serde_json::from_slice(&fs::read(&member_path).expect("copied character member must read"))
            .expect("copied character member must be JSON");
    member["source_revision"] =
        Value::String("1111111111111111111111111111111111111111".to_string());
    let mut member_bytes = serde_json::to_vec_pretty(&member).expect("member must serialize");
    member_bytes.push(b'\n');
    fs::write(&member_path, &member_bytes).expect("copied character member must update");

    let manifest_path = bundle.path().join("manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("copied manifest must read"))
            .expect("copied manifest must be JSON");
    manifest["files"]["characters.json"]["sha256"] =
        Value::String(format!("{:x}", Sha256::digest(&member_bytes)));
    manifest["files"]["characters.json"]["byte_count"] = Value::from(member_bytes.len() as u64);
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest must serialize");
    manifest_bytes.push(b'\n');
    fs::write(&manifest_path, manifest_bytes).expect("copied manifest must update");

    let error = ReferenceCache::from_provider(&GiloreBundleReferenceProvider::new(bundle.path()))
        .expect_err("mixed revisions must fail closed");
    assert_eq!(error.code(), "HSR-GILORE-COHERENCE");
    assert!(error
        .localized_message(hsr_scanner_experimental::Language::En)
        .contains("characters.json"));
}

fn update_manifest_file_metadata(
    root: &Path,
    filename: &str,
    byte_count: Option<u64>,
    entity_count: Option<u64>,
    sha256: Option<String>,
) {
    let manifest_path = root.join("manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("copied manifest must read"))
            .expect("copied manifest must be JSON");
    if let Some(byte_count) = byte_count {
        manifest["files"][filename]["byte_count"] = Value::from(byte_count);
    }
    if let Some(entity_count) = entity_count {
        manifest["files"][filename]["entity_count"] = Value::from(entity_count);
    }
    if let Some(sha256) = sha256 {
        manifest["files"][filename]["sha256"] = Value::from(sha256);
    }
    let mut bytes = serde_json::to_vec_pretty(&manifest).expect("manifest serialization");
    bytes.push(b'\n');
    fs::write(manifest_path, bytes).expect("manifest metadata update");
}

#[test]
fn provider_rejects_manifest_byte_and_entity_count_mismatches() {
    for (label, field, code) in [
        ("bad-byte-count", "byte_count", "HSR-GILORE-BYTE-COUNT"),
        (
            "bad-entity-count",
            "entity_count",
            "HSR-GILORE-ENTITY-COUNT",
        ),
    ] {
        let bundle = CopiedBundle::new(label);
        let manifest_path = bundle.path().join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).expect("copied manifest must read"))
                .expect("copied manifest must be JSON");
        let current = manifest["files"]["characters.json"][field]
            .as_u64()
            .expect("count field must be numeric");
        manifest["files"]["characters.json"][field] = Value::from(current + 1);
        let mut bytes = serde_json::to_vec_pretty(&manifest).expect("manifest serialization");
        bytes.push(b'\n');
        fs::write(&manifest_path, bytes).expect("manifest count mutation");

        let error =
            ReferenceCache::from_provider(&GiloreBundleReferenceProvider::new(bundle.path()))
                .expect_err("declared count drift must fail closed");
        assert_eq!(error.code(), code);
    }
}

#[test]
fn tiny_verified_bundle_is_fixture_capable_but_not_live_complete() {
    let cache = load_cache(fixture("gilore_bundle"));
    let observations = FixtureObservationSource::new(fixture("observations.json"))
        .load()
        .expect("tiny bundle remains usable for deterministic fixture export");
    build_export(observations, &cache).expect("fixture export remains allowed");

    let error = cache
        .validate_live_complete_profile()
        .expect_err("tiny fixture must never authorize an account-facing path");
    assert_eq!(error.code(), "HSR-REF-LIVE-INCOMPLETE");
    assert!(error
        .localized_message(hsr_scanner_experimental::Language::En)
        .contains("characters"));
}

struct CopiedBundle {
    root: PathBuf,
}

impl CopiedBundle {
    fn new(label: &str) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "goodscanner-hsr-gilore-{}-{unique}-{label}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("unique test directory must be created");
        let source = fixture("gilore_bundle");
        for name in BUNDLE_FILES {
            fs::copy(source.join(name), root.join(name)).expect("bundle member must copy");
        }
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for CopiedBundle {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
