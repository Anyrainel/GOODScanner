use std::cell::Cell;
use std::collections::{BTreeSet, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use hsr_scanner::localization::Language;
use hsr_scanner::manager::{
    apply_manager_envelope, apply_manager_plan, build_manager_plan,
    validate_manager_envelope_reference, AppendOnlyJsonJournalStore, ApplyAuthorization,
    JournalEntry, JournalStatus, ManagedGearObservation, ManagedState, ManagerInstructionsEnvelope,
    ManagerJournal, ManagerJournalStore, ManagerMutationDevice, MutationScope, PlanClassification,
    VisibleGearMatcher,
};
use hsr_scanner::reference::{GiloreBundleReferenceProvider, ReferenceCache};

const GOLDEN_MANAGER_INSTRUCTIONS: &str = include_str!("fixtures/manager_instructions_v1.json");
const GOLDEN_MANAGER_PREVIEW_EN: &str = include_str!("fixtures/manager_preview_en.txt");
const GOLDEN_MANAGER_PREVIEW_ZH_CN: &str = include_str!("fixtures/manager_preview_zh_cn.txt");
static APPLY_LEASE_TEST_SERIAL: Mutex<()> = Mutex::new(());

fn apply_lease_test_guard() -> std::sync::MutexGuard<'static, ()> {
    APPLY_LEASE_TEST_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn compact_json(input: &str) -> String {
    let mut compact = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    for character in input.chars() {
        if in_string {
            compact.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
        } else if character == '"' {
            in_string = true;
            compact.push(character);
        } else if !character.is_whitespace() {
            compact.push(character);
        }
    }
    compact
}

fn refresh_envelope_idempotency(envelope: &mut ManagerInstructionsEnvelope) {
    envelope.idempotency_key = envelope.expected_idempotency_key().unwrap();
}

fn refresh_idempotency_key(value: &mut serde_json::Value) {
    let envelope: ManagerInstructionsEnvelope = serde_json::from_value(value.clone()).unwrap();
    value["idempotencyKey"] =
        serde_json::Value::String(envelope.expected_idempotency_key().unwrap());
}

fn envelope() -> ManagerInstructionsEnvelope {
    ManagerInstructionsEnvelope::parse_json(GOLDEN_MANAGER_INSTRUCTIONS).unwrap()
}

fn two_instruction_envelope() -> ManagerInstructionsEnvelope {
    let mut envelope = envelope();
    let mut second = envelope.instructions[0].clone();
    second.id = "hsr-manager-0002-lock".to_owned();
    second.matcher.key = "63015".to_owned();
    second.matcher.game_id = 63015;
    second.matcher.set_key = "301".to_owned();
    second.matcher.slot = hsr_scanner::model::GearSlot::PlanarSphere;
    second.matcher.main_stat.key = "AttackAddedRatio".to_owned();
    second.matcher.main_stat.value = 43.2;
    envelope.instructions.push(second);
    refresh_envelope_idempotency(&mut envelope);
    envelope
}

fn two_before_observations(envelope: &ManagerInstructionsEnvelope) -> Vec<ManagedGearObservation> {
    envelope
        .instructions
        .iter()
        .map(|instruction| {
            observed(
                instruction.matcher.clone(),
                Some(false),
                Some(false),
                Some(false),
            )
        })
        .collect()
}

fn temporary_journal_path(test_name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "goodscanner-hsr-manager-{test_name}-{}-{nonce}.jsonl",
        std::process::id()
    ))
}

fn journal_lock_path(path: &std::path::Path) -> PathBuf {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".apply.lock");
    PathBuf::from(lock_path)
}

fn remove_journal_and_lock(path: &std::path::Path) {
    for candidate in [path.to_path_buf(), journal_lock_path(path)] {
        match fs::remove_file(candidate) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => panic!("failed to clean manager test file: {error}"),
        }
    }
}

fn observed(
    matcher: VisibleGearMatcher,
    lock: Option<bool>,
    discard: Option<bool>,
    equipped: Option<bool>,
) -> ManagedGearObservation {
    ManagedGearObservation {
        matcher,
        state: ManagedState { lock, discard },
        equipped,
    }
}

#[derive(Default)]
struct MemoryJournalStore {
    journal: Option<ManagerJournal>,
    saves: usize,
}

impl ManagerJournalStore for MemoryJournalStore {
    fn load(&mut self) -> Result<Option<ManagerJournal>, String> {
        Ok(self.journal.clone())
    }

    fn save(&mut self, journal: &ManagerJournal) -> Result<(), String> {
        self.journal = Some(journal.clone());
        self.saves += 1;
        Ok(())
    }

    fn holds_exclusive_apply_lease(&self) -> bool {
        true
    }
}

struct SimulatedDevice {
    matches: Vec<ManagedGearObservation>,
    rereads: usize,
    toggles: Vec<MutationScope>,
    fail_toggle_after_state_change: bool,
    duplicate_after_toggle: bool,
    panic_during_toggle: bool,
    preclick_journal_path: Option<PathBuf>,
    durable_preclick_seen: bool,
    reread_failures: HashSet<u32>,
}

impl SimulatedDevice {
    fn new(matches: Vec<ManagedGearObservation>) -> Self {
        Self {
            matches,
            rereads: 0,
            toggles: Vec::new(),
            fail_toggle_after_state_change: false,
            duplicate_after_toggle: false,
            panic_during_toggle: false,
            preclick_journal_path: None,
            durable_preclick_seen: false,
            reread_failures: HashSet::new(),
        }
    }
}

impl ManagerMutationDevice for SimulatedDevice {
    fn reread(
        &mut self,
        matcher: &VisibleGearMatcher,
    ) -> Result<Vec<ManagedGearObservation>, String> {
        self.rereads += 1;
        if self.reread_failures.contains(&matcher.game_id) {
            return Err("simulated private device detail".to_owned());
        }
        Ok(self
            .matches
            .iter()
            .filter(|observation| observation.matcher.game_id == matcher.game_id)
            .cloned()
            .collect())
    }

    fn toggle_once(
        &mut self,
        target: &ManagedGearObservation,
        scope: MutationScope,
    ) -> Result<(), String> {
        self.toggles.push(scope);
        if self.panic_during_toggle {
            panic!("simulated process interruption after durable mutationStarted");
        }
        if let Some(path) = self.preclick_journal_path.as_ref() {
            let mut store = AppendOnlyJsonJournalStore::new(path);
            let journal = store
                .load()
                .expect("pre-click journal must be reloadable")
                .expect("pre-click journal must exist");
            assert_eq!(journal.entries[0].status, JournalStatus::MutationStarted);
            assert_eq!(journal.entries[0].toggle_attempts, 1);
            self.durable_preclick_seen = true;
        }
        let target_index = self
            .matches
            .iter()
            .position(|observation| observation.matcher.game_id == target.matcher.game_id)
            .expect("simulated target must remain present");
        let state = &mut self.matches[target_index].state;
        match scope {
            MutationScope::Lock => state.lock = Some(true),
            MutationScope::Unlock => state.lock = Some(false),
            MutationScope::MarkDiscard => state.discard = Some(true),
            MutationScope::UnmarkDiscard => state.discard = Some(false),
        }
        if self.duplicate_after_toggle {
            self.matches.push(self.matches[target_index].clone());
            self.duplicate_after_toggle = false;
        }
        if self.fail_toggle_after_state_change {
            Err("simulated device transport error with secret-like text".to_owned())
        } else {
            Ok(())
        }
    }
}

#[test]
fn exact_manager_v1_golden_contract_round_trips_without_stat_game_ids() {
    let parsed = envelope();
    assert_eq!(parsed.schema, "goodscanner.hsr.manager-instructions");
    assert_eq!(parsed.schema_version, 1);
    assert!(!parsed.privacy.account_identifiers_included);
    assert!(!parsed.privacy.raw_packet_data_included);
    assert!(!parsed.privacy.server_item_identifiers_included);
    assert_eq!(parsed.instructions[0].matcher.game_id, 61011);
    assert_eq!(parsed.instructions[0].matcher.main_stat.key, "HPDelta");

    let exact_serialized = serde_json::to_string(&parsed).unwrap();
    assert_eq!(exact_serialized, compact_json(GOLDEN_MANAGER_INSTRUCTIONS));
    assert!(exact_serialized.contains("\"id\":\"hsr-manager-0001-lock\""));

    let serialized = serde_json::to_value(parsed).unwrap();
    assert!(serialized["instructions"][0]["matcher"]["mainStat"]
        .get("gameId")
        .is_none());
    assert!(serialized["instructions"][0]["matcher"]["substats"][0]
        .get("gameId")
        .is_none());
    assert!(serialized["instructions"][0]["desired"]
        .get("discard")
        .is_none());
}

#[test]
fn idempotency_key_is_verified_and_excludes_request_id() {
    let mut changed_request: serde_json::Value =
        serde_json::from_str(GOLDEN_MANAGER_INSTRUCTIONS).unwrap();
    changed_request["requestId"] = serde_json::json!("another-valid-request:2");
    ManagerInstructionsEnvelope::parse_json(&changed_request.to_string())
        .expect("requestId must be excluded from the semantic digest");

    changed_request["idempotencyKey"] = serde_json::json!(
        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
    );
    let error = ManagerInstructionsEnvelope::parse_json(&changed_request.to_string())
        .expect_err("a syntactically valid but incorrect digest must be rejected");
    assert_eq!(error.code(), "HSR_MANAGER_IDEMPOTENCY_KEY_MISMATCH");
}

#[test]
fn idempotency_canonicalizes_multi_instruction_order() {
    let original = two_instruction_envelope();
    original.validate().unwrap();
    let mut reordered = original.clone();
    reordered.instructions.reverse();

    assert_ne!(
        serde_json::to_string(&original.instructions).unwrap(),
        serde_json::to_string(&reordered.instructions).unwrap()
    );
    assert_eq!(original.idempotency_key, reordered.idempotency_key);
    reordered
        .validate()
        .expect("wire order must not change semantic idempotency");
}

#[test]
fn idempotency_matches_javascript_stringify_for_integral_stat_values() {
    let mut integral = envelope();
    integral.instructions[0].matcher.substats[1].value = 5.0;
    assert_eq!(
        integral.expected_idempotency_key().unwrap(),
        "sha256:06cfe8338317840d890ca2d878879927d2e15e9a60f40a67f7276b02cf5eaf45"
    );
    assert!(serde_json::to_string(&integral.instructions)
        .unwrap()
        .contains("\"value\":5}"));
}

#[test]
fn website_request_id_rarity_and_finite_stat_boundaries_are_accepted_exactly() {
    let mut value: serde_json::Value = serde_json::from_str(GOLDEN_MANAGER_INSTRUCTIONS).unwrap();
    value["requestId"] = serde_json::json!("A.valid_request-id:1");
    value["instructions"][0]["matcher"]["rarity"] = serde_json::json!(1);
    value["instructions"][0]["matcher"]["mainStat"]["value"] = serde_json::json!(-1.0);
    refresh_idempotency_key(&mut value);
    ManagerInstructionsEnvelope::parse_json(&value.to_string())
        .expect("the website schema accepts rarity 1 and finite negative stat values");

    for request_id in ["-starts-with-punctuation".to_owned(), "a".repeat(129)] {
        let mut invalid = value.clone();
        invalid["requestId"] = serde_json::Value::String(request_id);
        assert!(ManagerInstructionsEnvelope::parse_json(&invalid.to_string()).is_err());
    }
}

#[test]
fn request_and_instruction_ids_are_terminal_safe_ascii_with_a_128_byte_limit() {
    let mut valid = envelope();
    valid.request_id = format!("A{}", "r".repeat(127));
    valid.instructions[0].id = format!("I{}", "d".repeat(127));
    refresh_envelope_idempotency(&mut valid);
    valid.validate().expect("128 safe ASCII bytes are accepted");

    for unsafe_id in [
        "line\nspoof".to_owned(),
        "tab\tspoof".to_owned(),
        "é".to_owned(),
        "-starts-with-punctuation".to_owned(),
        format!("I{}", "d".repeat(128)),
    ] {
        let mut invalid_instruction = envelope();
        invalid_instruction.instructions[0].id = unsafe_id.clone();
        refresh_envelope_idempotency(&mut invalid_instruction);
        assert!(invalid_instruction.validate().is_err(), "{unsafe_id:?}");

        let mut invalid_request = envelope();
        invalid_request.request_id = unsafe_id;
        assert!(invalid_request.validate().is_err());
    }
}

#[test]
fn envelope_reference_validator_fails_closed_before_any_device_boundary() {
    let bundle = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("gilore_bundle");
    let reference = ReferenceCache::from_provider(&GiloreBundleReferenceProvider::new(bundle))
        .expect("the checked-in normalized GIlore bundle must load");
    let canonical = envelope();
    validate_manager_envelope_reference(&canonical, &reference)
        .expect("the GGStarRail golden must equal the normalized reference");

    let mut revision_drift = canonical.clone();
    revision_drift.reference.revision = "different-public-revision".to_owned();
    refresh_envelope_idempotency(&mut revision_drift);
    let error = validate_manager_envelope_reference(&revision_drift, &reference).unwrap_err();
    assert_eq!(error.code(), "HSR_MANAGER_REFERENCE_MISMATCH");

    let mut piece_drift = canonical.clone();
    piece_drift.instructions[0].matcher.set_key = "999".to_owned();
    refresh_envelope_idempotency(&mut piece_drift);
    assert!(validate_manager_envelope_reference(&piece_drift, &reference).is_err());

    let mut progression_drift = canonical;
    progression_drift.instructions[0].matcher.main_stat.value = 705.7;
    refresh_envelope_idempotency(&mut progression_drift);
    assert!(validate_manager_envelope_reference(&progression_drift, &reference).is_err());
}

#[test]
fn strict_contract_rejects_unknown_fields_sensitive_ids_and_empty_desired() {
    let mut value: serde_json::Value = serde_json::from_str(GOLDEN_MANAGER_INSTRUCTIONS).unwrap();
    value["instructions"][0]["serverItemId"] = serde_json::json!("private-123");
    let error = ManagerInstructionsEnvelope::parse_json(&value.to_string()).unwrap_err();
    assert_eq!(error.code(), "HSR_MANAGER_SENSITIVE_FIELD");
    assert!(!error.to_string().contains("private-123"));

    let mut value: serde_json::Value = serde_json::from_str(GOLDEN_MANAGER_INSTRUCTIONS).unwrap();
    value["instructions"][0]["unexpected"] = serde_json::json!(true);
    assert!(ManagerInstructionsEnvelope::parse_json(&value.to_string()).is_err());

    let mut value: serde_json::Value = serde_json::from_str(GOLDEN_MANAGER_INSTRUCTIONS).unwrap();
    value["instructions"][0]["desired"] = serde_json::json!({});
    assert!(ManagerInstructionsEnvelope::parse_json(&value.to_string()).is_err());
}

#[test]
fn planner_classifies_action_noop_unknown_equipped_not_found_and_ambiguity() {
    let base = envelope();
    let matcher = base.instructions[0].matcher.clone();

    let action = build_manager_plan(
        &base,
        &[observed(
            matcher.clone(),
            Some(false),
            Some(false),
            Some(false),
        )],
    )
    .unwrap();
    assert_eq!(
        action.entries[0].classification,
        PlanClassification::Actionable
    );
    assert_eq!(action.entries[0].changes.len(), 1);
    assert_eq!(action.entries[0].changes[0].scope, MutationScope::Lock);

    let noop = build_manager_plan(
        &base,
        &[observed(
            matcher.clone(),
            Some(true),
            Some(false),
            Some(false),
        )],
    )
    .unwrap();
    assert_eq!(
        noop.entries[0].classification,
        PlanClassification::NoOpAlreadyDesired
    );

    let mut unknown_before = base.clone();
    unknown_before.instructions[0].before.lock = None;
    refresh_envelope_idempotency(&mut unknown_before);
    let unknown = build_manager_plan(
        &unknown_before,
        &[observed(
            matcher.clone(),
            Some(false),
            Some(false),
            Some(false),
        )],
    )
    .unwrap();
    assert_eq!(
        unknown.entries[0].classification,
        PlanClassification::PreviewOnlyUnknownBefore
    );

    let mut stale_before = base.clone();
    stale_before.instructions[0].before.lock = Some(true);
    refresh_envelope_idempotency(&mut stale_before);
    let stale = build_manager_plan(
        &stale_before,
        &[observed(
            matcher.clone(),
            Some(false),
            Some(false),
            Some(false),
        )],
    )
    .unwrap();
    assert_eq!(
        stale.entries[0].classification,
        PlanClassification::PreviewOnlyBeforeMismatch
    );

    let equipment_unknown = build_manager_plan(
        &base,
        &[observed(matcher.clone(), Some(false), Some(false), None)],
    )
    .unwrap();
    assert_eq!(
        equipment_unknown.entries[0].classification,
        PlanClassification::PreviewOnlyEquipmentUnknown
    );

    let equipped = build_manager_plan(
        &base,
        &[observed(
            VisibleGearMatcher {
                location_key: Some("1001".to_owned()),
                ..matcher.clone()
            },
            Some(false),
            Some(false),
            Some(true),
        )],
    )
    .unwrap();
    assert_eq!(
        equipped.entries[0].classification,
        PlanClassification::PreviewOnlyEquipped
    );

    let not_found = build_manager_plan(&base, &[]).unwrap();
    assert_eq!(
        not_found.entries[0].classification,
        PlanClassification::PreviewOnlyNotFound
    );

    let ambiguous = build_manager_plan(
        &base,
        &[
            observed(matcher.clone(), Some(false), Some(false), Some(false)),
            observed(matcher, Some(false), Some(false), Some(false)),
        ],
    )
    .unwrap();
    assert_eq!(
        ambiguous.entries[0].classification,
        PlanClassification::PreviewOnlyAmbiguous
    );
}

#[test]
fn planner_requires_both_before_flags_and_rejects_collateral_drift() {
    let base = envelope();
    let matcher = base.instructions[0].matcher.clone();

    let collateral_drift = build_manager_plan(
        &base,
        &[observed(
            matcher.clone(),
            Some(false),
            Some(true),
            Some(false),
        )],
    )
    .unwrap();
    assert_eq!(
        collateral_drift.entries[0].classification,
        PlanClassification::PreviewOnlyBeforeMismatch
    );
    assert!(collateral_drift.entries[0].changes.is_empty());

    let mut missing_collateral_before = base;
    missing_collateral_before.instructions[0].before.discard = None;
    refresh_envelope_idempotency(&mut missing_collateral_before);
    let unknown = build_manager_plan(
        &missing_collateral_before,
        &[observed(matcher, Some(false), Some(false), Some(false))],
    )
    .unwrap();
    assert_eq!(
        unknown.entries[0].classification,
        PlanClassification::PreviewOnlyUnknownBefore
    );
}

#[test]
fn planner_keeps_mark_discard_on_locked_relic_preview_only() {
    let mut locked_discard = envelope();
    locked_discard.instructions[0].before.lock = Some(true);
    locked_discard.instructions[0].before.discard = Some(false);
    locked_discard.instructions[0].desired.lock = None;
    locked_discard.instructions[0].desired.discard = Some(true);
    refresh_envelope_idempotency(&mut locked_discard);
    let inventory = [observed(
        locked_discard.instructions[0].matcher.clone(),
        Some(true),
        Some(false),
        Some(false),
    )];

    let plan = build_manager_plan(&locked_discard, &inventory).unwrap();

    assert_eq!(
        plan.entries[0].classification,
        PlanClassification::PreviewOnlyLocked
    );
    assert!(plan.entries[0].changes.is_empty());
    assert!(plan.required_scopes().is_empty());
    assert_eq!(
        serde_json::to_value(&plan).unwrap()["entries"][0]["classification"],
        "previewOnlyLocked"
    );
    assert!(plan
        .render(Language::En)
        .contains("preview only: gear is locked"));
    assert!(plan.render(Language::ZhCn).contains("仅预览：装备已锁定"));
}

#[test]
fn declared_locked_discard_stays_preview_only_through_apply_and_restart() {
    let _serial = apply_lease_test_guard();
    let mut locked_discard = envelope();
    locked_discard.instructions[0].before.lock = Some(true);
    locked_discard.instructions[0].before.discard = Some(false);
    locked_discard.instructions[0].desired.lock = None;
    locked_discard.instructions[0].desired.discard = Some(true);
    refresh_envelope_idempotency(&mut locked_discard);
    let planning = observed(
        locked_discard.instructions[0].matcher.clone(),
        Some(true),
        Some(false),
        Some(false),
    );
    let preview = build_manager_plan(&locked_discard, std::slice::from_ref(&planning)).unwrap();
    let authorization = ApplyAuthorization::new(&preview.digest, [MutationScope::MarkDiscard]);
    let path = temporary_journal_path("declared-locked-discard");
    let scans = Cell::new(0);
    let reviews = Cell::new(0);
    let mut device = SimulatedDevice::new(vec![planning.clone()]);

    let report = {
        let store = AppendOnlyJsonJournalStore::new(&path);
        let mut lease = store.try_acquire_apply_lease().unwrap();
        apply_manager_envelope(
            &locked_discard,
            &authorization,
            &mut device,
            &mut lease,
            |_| {
                scans.set(scans.get() + 1);
                Ok(vec![planning.clone()])
            },
            |exact| {
                reviews.set(reviews.get() + 1);
                assert_eq!(
                    exact.entries[0].classification,
                    PlanClassification::PreviewOnlyLocked
                );
                assert!(exact.entries[0].changes.is_empty());
                Ok(())
            },
        )
        .unwrap()
    };

    assert_eq!(
        scans.get(),
        1,
        "a first apply still obtains fresh preview evidence"
    );
    assert_eq!(
        reviews.get(),
        1,
        "the exact preview must be displayed before apply"
    );
    assert_eq!(report.total_actions, 0);
    assert_eq!(report.device_toggles, 0);
    assert_eq!(device.rereads, 0);
    assert!(device.toggles.is_empty());

    let mut reloaded = AppendOnlyJsonJournalStore::new(&path);
    let journal = reloaded.load().unwrap().unwrap();
    assert!(journal.entries.is_empty());
    assert_eq!(
        journal.plan.entries[0].classification,
        PlanClassification::PreviewOnlyLocked
    );

    let restart_scans = Cell::new(0);
    let mut restarted_device = SimulatedDevice::new(Vec::new());
    let restarted = {
        let store = AppendOnlyJsonJournalStore::new(&path);
        let mut lease = store.try_acquire_apply_lease().unwrap();
        apply_manager_envelope(
            &locked_discard,
            &authorization,
            &mut restarted_device,
            &mut lease,
            |_| {
                restart_scans.set(restart_scans.get() + 1);
                panic!("a restart must recover the empty preview-only plan before device access")
            },
            |exact| {
                assert_eq!(
                    exact.entries[0].classification,
                    PlanClassification::PreviewOnlyLocked
                );
                assert!(exact.entries[0].changes.is_empty());
                Ok(())
            },
        )
        .unwrap()
    };
    assert_eq!(restart_scans.get(), 0);
    assert_eq!(restarted.total_actions, 0);
    assert_eq!(restarted.device_toggles, 0);
    assert_eq!(restarted_device.rereads, 0);
    assert!(restarted_device.toggles.is_empty());
    remove_journal_and_lock(&path);
}

#[test]
fn location_is_optional_for_matching_but_never_bypasses_equipped_protection() {
    let base = envelope();
    assert!(base.instructions[0].matcher.location_key.is_none());
    let equipped_matcher = VisibleGearMatcher {
        location_key: Some("1001".to_owned()),
        ..base.instructions[0].matcher.clone()
    };
    let plan = build_manager_plan(
        &base,
        &[observed(
            equipped_matcher,
            Some(false),
            Some(false),
            Some(true),
        )],
    )
    .unwrap();
    assert_eq!(
        plan.entries[0].classification,
        PlanClassification::PreviewOnlyEquipped
    );
    assert!(plan.entries[0].changes.is_empty());
}

#[test]
fn apply_requires_exact_digest_and_individual_action_scope() {
    let envelope = envelope();
    let observation = observed(
        envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let plan = build_manager_plan(&envelope, std::slice::from_ref(&observation)).unwrap();
    let mut device = SimulatedDevice::new(vec![observation]);
    let mut store = MemoryJournalStore::default();

    let wrong_digest = ApplyAuthorization::new("wrong", [MutationScope::Lock]);
    assert!(apply_manager_plan(&plan, &wrong_digest, &mut device, &mut store).is_err());
    assert!(device.toggles.is_empty());

    let missing_scope = ApplyAuthorization::new(&plan.digest, []);
    assert!(apply_manager_plan(&plan, &missing_scope, &mut device, &mut store).is_err());
    assert!(device.toggles.is_empty());
    assert!(store.journal.is_none());
}

#[test]
fn apply_rejects_an_append_only_store_without_its_exclusive_lease() {
    let envelope = envelope();
    let observation = observed(
        envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let plan = build_manager_plan(&envelope, std::slice::from_ref(&observation)).unwrap();
    let authorization = ApplyAuthorization::new(&plan.digest, [MutationScope::Lock]);
    let path = temporary_journal_path("unlocked-store");
    let mut unlocked = AppendOnlyJsonJournalStore::new(&path);
    let mut device = SimulatedDevice::new(vec![observation]);

    let error = apply_manager_plan(&plan, &authorization, &mut device, &mut unlocked).unwrap_err();
    assert_eq!(error.code(), "HSR_MANAGER_EXCLUSIVE_LEASE_REQUIRED");
    assert!(device.toggles.is_empty());
    assert!(!path.exists());
    remove_journal_and_lock(&path);
}

#[test]
fn apply_rejects_plan_content_changed_after_preview() {
    let envelope = envelope();
    let observation = observed(
        envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let mut plan = build_manager_plan(&envelope, std::slice::from_ref(&observation)).unwrap();
    let authorization = ApplyAuthorization::new(&plan.digest, [MutationScope::Lock]);
    plan.entries[0].changes[0].desired = false;
    let mut device = SimulatedDevice::new(vec![observation]);
    let mut store = MemoryJournalStore::default();
    let error = apply_manager_plan(&plan, &authorization, &mut device, &mut store).unwrap_err();
    assert_eq!(error.code(), "HSR_MANAGER_PLAN_DIGEST_INVALID");
    assert!(device.toggles.is_empty());
    assert!(store.journal.is_none());
}

#[test]
fn apply_toggles_once_postverifies_and_is_idempotent_on_repeat() {
    let envelope = envelope();
    let observation = observed(
        envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let plan = build_manager_plan(&envelope, std::slice::from_ref(&observation)).unwrap();
    let authorization = ApplyAuthorization::new(&plan.digest, [MutationScope::Lock]);
    let mut device = SimulatedDevice::new(vec![observation]);
    let mut store = MemoryJournalStore::default();

    let first = apply_manager_plan(&plan, &authorization, &mut device, &mut store).unwrap();
    assert_eq!(first.verified_actions, 1);
    assert_eq!(first.needs_review_actions, 0);
    assert_eq!(first.device_toggles, 1);
    assert_eq!(device.toggles, vec![MutationScope::Lock]);
    assert!(store.saves >= 3); // initialized, pre-click, post-verification

    let second = apply_manager_plan(&plan, &authorization, &mut device, &mut store).unwrap();
    assert_eq!(second.verified_actions, 1);
    assert_eq!(second.device_toggles, 0);
    assert_eq!(device.toggles, vec![MutationScope::Lock]);
}

#[test]
fn journal_first_cli_retry_uses_original_plan_and_skips_a_second_full_scan() {
    let _serial = apply_lease_test_guard();
    let original_envelope = envelope();
    let planning = observed(
        original_envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let preview = build_manager_plan(&original_envelope, std::slice::from_ref(&planning)).unwrap();
    let authorization = ApplyAuthorization::new(&preview.digest, [MutationScope::Lock]);
    let path = temporary_journal_path("cli-retry");

    let first_scan_count = Cell::new(0);
    let mut first_device = SimulatedDevice::new(vec![planning.clone()]);
    {
        let store = AppendOnlyJsonJournalStore::new(&path);
        let mut lease = store.try_acquire_apply_lease().unwrap();
        let report = apply_manager_envelope(
            &original_envelope,
            &authorization,
            &mut first_device,
            &mut lease,
            |_| {
                first_scan_count.set(first_scan_count.get() + 1);
                Ok(vec![planning.clone()])
            },
            |plan| {
                assert_eq!(plan.digest, preview.digest);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(report.verified_actions, 1);
        assert_eq!(report.original_request_id, original_envelope.request_id);
    }
    assert_eq!(first_scan_count.get(), 1);
    assert_eq!(first_device.toggles, vec![MutationScope::Lock]);

    // GGStarRail may assign a new correlation ID to the same semantic retry.
    // The settled idempotency key excludes requestId, so recovery must use the
    // original plan/digest while reporting both IDs.
    let mut retry_envelope = original_envelope.clone();
    retry_envelope.request_id = "retry-request-2".to_owned();
    retry_envelope.validate().unwrap();
    let retry_scan_count = Cell::new(0);
    let mut retry_device = SimulatedDevice::new(Vec::new());
    {
        let store = AppendOnlyJsonJournalStore::new(&path);
        let mut lease = store.try_acquire_apply_lease().unwrap();
        let report = apply_manager_envelope(
            &retry_envelope,
            &authorization,
            &mut retry_device,
            &mut lease,
            |_| {
                retry_scan_count.set(retry_scan_count.get() + 1);
                panic!("completed retry must not rebuild inventory or its digest")
            },
            |plan| {
                assert_eq!(plan.digest, preview.digest);
                assert_eq!(plan.request_id, original_envelope.request_id);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(report.verified_actions, 1);
        assert_eq!(report.device_toggles, 0);
        assert_eq!(report.original_request_id, original_envelope.request_id);
        assert_eq!(report.submitted_request_id, retry_envelope.request_id);
    }
    assert_eq!(retry_scan_count.get(), 0);
    assert!(retry_device.toggles.is_empty());

    for changed in [
        {
            let mut changed = retry_envelope.clone();
            changed.instructions[0].desired.lock = Some(false);
            refresh_envelope_idempotency(&mut changed);
            changed
        },
        {
            let mut changed = retry_envelope.clone();
            changed.reference.revision = "different-public-revision".to_owned();
            refresh_envelope_idempotency(&mut changed);
            changed
        },
    ] {
        let store = AppendOnlyJsonJournalStore::new(&path);
        let mut lease = store.try_acquire_apply_lease().unwrap();
        let mut device = SimulatedDevice::new(Vec::new());
        let error = apply_manager_envelope(
            &changed,
            &authorization,
            &mut device,
            &mut lease,
            |_| panic!("changed semantics/reference must fail before a full scan"),
            |_| panic!("changed semantics/reference must fail before preview"),
        )
        .unwrap_err();
        assert_eq!(error.code(), "HSR_MANAGER_JOURNAL_ENVELOPE_MISMATCH");
        assert!(device.toggles.is_empty());
    }

    remove_journal_and_lock(&path);
}

#[test]
fn journal_first_process_restart_reconciles_mutation_started_without_rescan_or_retry() {
    let _serial = apply_lease_test_guard();
    let original_envelope = envelope();
    let planning = observed(
        original_envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let preview = build_manager_plan(&original_envelope, std::slice::from_ref(&planning)).unwrap();
    let authorization = ApplyAuthorization::new(&preview.digest, [MutationScope::Lock]);
    let path = temporary_journal_path("process-restart");
    let mut interrupted_device = SimulatedDevice::new(vec![planning.clone()]);
    interrupted_device.panic_during_toggle = true;

    {
        let store = AppendOnlyJsonJournalStore::new(&path);
        let mut lease = store.try_acquire_apply_lease().unwrap();
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = apply_manager_envelope(
                &original_envelope,
                &authorization,
                &mut interrupted_device,
                &mut lease,
                |_| Ok(vec![planning.clone()]),
                |_| Ok(()),
            );
        }));
        assert!(interrupted.is_err());
    }

    let mut retry_envelope = original_envelope.clone();
    retry_envelope.request_id = "retry-after-interruption".to_owned();
    let restart_scan_count = Cell::new(0);
    let mut restarted_device = SimulatedDevice::new(vec![observed(
        original_envelope.instructions[0].matcher.clone(),
        Some(true),
        Some(false),
        Some(false),
    )]);
    {
        let store = AppendOnlyJsonJournalStore::new(&path);
        let mut lease = store.try_acquire_apply_lease().unwrap();
        let report = apply_manager_envelope(
            &retry_envelope,
            &authorization,
            &mut restarted_device,
            &mut lease,
            |_| {
                restart_scan_count.set(restart_scan_count.get() + 1);
                panic!("mutationStarted recovery must use the persisted original plan")
            },
            |plan| {
                assert_eq!(plan.digest, preview.digest);
                assert_eq!(plan.request_id, original_envelope.request_id);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(report.verified_actions, 1);
        assert_eq!(report.device_toggles, 0);
        assert_eq!(report.original_request_id, original_envelope.request_id);
        assert_eq!(report.submitted_request_id, retry_envelope.request_id);
    }
    assert_eq!(restart_scan_count.get(), 0);
    assert!(restarted_device.toggles.is_empty());

    remove_journal_and_lock(&path);
}

#[test]
fn device_error_is_postverified_without_leaking_device_detail_into_journal() {
    let envelope = envelope();
    let observation = observed(
        envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let plan = build_manager_plan(&envelope, std::slice::from_ref(&observation)).unwrap();
    let authorization = ApplyAuthorization::new(&plan.digest, [MutationScope::Lock]);
    let mut device = SimulatedDevice::new(vec![observation]);
    device.fail_toggle_after_state_change = true;
    let mut store = MemoryJournalStore::default();

    let report = apply_manager_plan(&plan, &authorization, &mut device, &mut store).unwrap();
    assert_eq!(report.verified_actions, 1);
    let serialized = serde_json::to_string(store.journal.as_ref().unwrap()).unwrap();
    assert!(serialized.contains("postverified_after_device_error"));
    assert!(!serialized.contains("secret-like"));
}

#[test]
fn interrupted_action_recovers_desired_state_but_never_retries_old_state() {
    let envelope = envelope();
    let before_observation = observed(
        envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let plan = build_manager_plan(&envelope, std::slice::from_ref(&before_observation)).unwrap();
    let authorization = ApplyAuthorization::new(&plan.digest, [MutationScope::Lock]);
    let interrupted = ManagerJournal {
        schema: "goodscanner.hsr.manager-journal".to_owned(),
        schema_version: 2,
        request_id: plan.request_id.clone(),
        plan_digest: plan.digest.clone(),
        plan: plan.clone(),
        entries: vec![JournalEntry {
            instruction_id: plan.entries[0].instruction_id.clone(),
            change: plan.entries[0].changes[0].clone(),
            status: JournalStatus::MutationStarted,
            toggle_attempts: 1,
            outcome_code: None,
        }],
        idempotency_key: plan.idempotency_key.clone(),
    };

    let mut desired_device = SimulatedDevice::new(vec![observed(
        envelope.instructions[0].matcher.clone(),
        Some(true),
        Some(false),
        Some(false),
    )]);
    let mut desired_store = MemoryJournalStore {
        journal: Some(interrupted.clone()),
        saves: 0,
    };
    let recovered = apply_manager_plan(
        &plan,
        &authorization,
        &mut desired_device,
        &mut desired_store,
    )
    .unwrap();
    assert_eq!(recovered.verified_actions, 1);
    assert!(desired_device.toggles.is_empty());

    let mut old_state_device = SimulatedDevice::new(vec![before_observation]);
    let mut old_state_store = MemoryJournalStore {
        journal: Some(interrupted),
        saves: 0,
    };
    let stopped = apply_manager_plan(
        &plan,
        &authorization,
        &mut old_state_device,
        &mut old_state_store,
    )
    .unwrap();
    assert_eq!(stopped.needs_review_actions, 1);
    assert!(old_state_device.toggles.is_empty());
    assert_eq!(
        old_state_store.journal.unwrap().entries[0]
            .outcome_code
            .as_deref(),
        Some("interrupted_mutation_ambiguous")
    );
}

#[test]
fn journal_rejects_impossible_status_attempt_and_outcome_combinations() {
    let envelope = envelope();
    let before = observed(
        envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let plan = build_manager_plan(&envelope, std::slice::from_ref(&before)).unwrap();
    let authorization = ApplyAuthorization::new(&plan.digest, [MutationScope::Lock]);
    let baseline = JournalEntry {
        instruction_id: plan.entries[0].instruction_id.clone(),
        change: plan.entries[0].changes[0].clone(),
        status: JournalStatus::Pending,
        toggle_attempts: 0,
        outcome_code: None,
    };

    let invalid_states = [
        (JournalStatus::Pending, 0, Some("unexpected")),
        (JournalStatus::Pending, 1, None),
        (JournalStatus::MutationStarted, 0, None),
        (JournalStatus::MutationStarted, 1, Some("postverified")),
        (JournalStatus::Verified, 0, None),
        (JournalStatus::Verified, 0, Some("postverified")),
        (
            JournalStatus::Verified,
            1,
            Some("already_desired_on_reread"),
        ),
        (JournalStatus::NeedsReview, 0, None),
        (
            JournalStatus::NeedsReview,
            0,
            Some("postverification_failed"),
        ),
        (JournalStatus::NeedsReview, 1, Some("fresh_match_not_found")),
        (
            JournalStatus::NeedsReview,
            2,
            Some("postverification_failed"),
        ),
    ];
    for (status, attempts, outcome) in invalid_states {
        let mut entry = baseline.clone();
        entry.status = status;
        entry.toggle_attempts = attempts;
        entry.outcome_code = outcome.map(str::to_owned);
        let journal = ManagerJournal {
            schema: "goodscanner.hsr.manager-journal".to_owned(),
            schema_version: 2,
            request_id: plan.request_id.clone(),
            idempotency_key: plan.idempotency_key.clone(),
            plan_digest: plan.digest.clone(),
            plan: plan.clone(),
            entries: vec![entry],
        };
        let mut store = MemoryJournalStore {
            journal: Some(journal),
            saves: 0,
        };
        let mut device = SimulatedDevice::new(vec![before.clone()]);
        let error = apply_manager_plan(&plan, &authorization, &mut device, &mut store)
            .expect_err("invalid journal state must fail before any device operation");
        assert_eq!(error.code(), "HSR_MANAGER_JOURNAL_STATUS_INVALID");
        assert!(device.toggles.is_empty());
    }
}

#[test]
fn append_only_store_recovers_truncated_tail_and_rejects_corrupt_complete_line() {
    let envelope = envelope();
    let before = observed(
        envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let plan = build_manager_plan(&envelope, &[before]).unwrap();
    let journal = ManagerJournal {
        schema: "goodscanner.hsr.manager-journal".to_owned(),
        schema_version: 2,
        request_id: plan.request_id.clone(),
        idempotency_key: plan.idempotency_key.clone(),
        plan_digest: plan.digest.clone(),
        plan: plan.clone(),
        entries: vec![JournalEntry {
            instruction_id: plan.entries[0].instruction_id.clone(),
            change: plan.entries[0].changes[0].clone(),
            status: JournalStatus::Pending,
            toggle_attempts: 0,
            outcome_code: None,
        }],
    };

    let truncated_path = temporary_journal_path("truncated");
    let mut truncated_store = AppendOnlyJsonJournalStore::new(&truncated_path);
    truncated_store.save(&journal).unwrap();
    let mut file = OpenOptions::new()
        .append(true)
        .open(&truncated_path)
        .unwrap();
    file.write_all(b"{\"schema\":\"goodscanner.hsr.manager-journal\"")
        .unwrap();
    file.flush().unwrap();
    drop(file);
    assert_eq!(truncated_store.load().unwrap(), Some(journal.clone()));
    fs::remove_file(&truncated_path).unwrap();

    let corrupt_path = temporary_journal_path("corrupt");
    let mut corrupt_store = AppendOnlyJsonJournalStore::new(&corrupt_path);
    corrupt_store.save(&journal).unwrap();
    let mut file = OpenOptions::new().append(true).open(&corrupt_path).unwrap();
    file.write_all(b"this-is-not-json\n").unwrap();
    file.flush().unwrap();
    drop(file);
    assert!(corrupt_store.load().is_err());
    fs::remove_file(&corrupt_path).unwrap();

    let rollback_path = temporary_journal_path("rollback");
    let mut rollback_store = AppendOnlyJsonJournalStore::new(&rollback_path);
    rollback_store.save(&journal).unwrap();
    let mut started = journal.clone();
    started.entries[0].status = JournalStatus::MutationStarted;
    started.entries[0].toggle_attempts = 1;
    rollback_store.save(&started).unwrap();
    rollback_store.save(&journal).unwrap();
    assert!(rollback_store.load().is_err());
    fs::remove_file(&rollback_path).unwrap();
}

#[test]
fn leased_append_repairs_truncated_tail_and_reloads_mutation_started_before_click() {
    let _serial = apply_lease_test_guard();
    let envelope = envelope();
    let before = observed(
        envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let plan = build_manager_plan(&envelope, std::slice::from_ref(&before)).unwrap();
    let authorization = ApplyAuthorization::new(&plan.digest, [MutationScope::Lock]);
    let pending = ManagerJournal {
        schema: "goodscanner.hsr.manager-journal".to_owned(),
        schema_version: 2,
        request_id: plan.request_id.clone(),
        idempotency_key: plan.idempotency_key.clone(),
        plan_digest: plan.digest.clone(),
        plan: plan.clone(),
        entries: vec![JournalEntry {
            instruction_id: plan.entries[0].instruction_id.clone(),
            change: plan.entries[0].changes[0].clone(),
            status: JournalStatus::Pending,
            toggle_attempts: 0,
            outcome_code: None,
        }],
    };
    let path = temporary_journal_path("repair-before-click");
    let mut seed_store = AppendOnlyJsonJournalStore::new(&path);
    seed_store.save(&pending).unwrap();
    let mut damaged = OpenOptions::new().append(true).open(&path).unwrap();
    damaged
        .write_all(b"{\"schema\":\"unterminated-tail")
        .unwrap();
    damaged.flush().unwrap();
    drop(damaged);
    assert_eq!(seed_store.load().unwrap(), Some(pending));

    let mut device = SimulatedDevice::new(vec![before]);
    device.preclick_journal_path = Some(path.clone());
    {
        let mut lease = AppendOnlyJsonJournalStore::new(&path)
            .try_acquire_apply_lease()
            .unwrap();
        let report = apply_manager_plan(&plan, &authorization, &mut device, &mut lease).unwrap();
        assert_eq!(report.verified_actions, 1);
    }
    assert!(device.durable_preclick_seen);
    assert_eq!(device.toggles, vec![MutationScope::Lock]);

    let contents = fs::read_to_string(&path).unwrap();
    assert!(!contents.contains("unterminated-tail"));
    assert_eq!(contents.lines().count(), 3); // pending, mutationStarted, verified
    let mut reloaded = AppendOnlyJsonJournalStore::new(&path);
    let terminal = reloaded.load().unwrap().unwrap();
    assert_eq!(terminal.entries[0].status, JournalStatus::Verified);
    remove_journal_and_lock(&path);
}

#[test]
fn journal_apply_lease_child_probe() {
    let Some(path) = std::env::var_os("GOODSCANNER_HSR_LOCK_PROBE_PATH") else {
        return;
    };
    let expectation = std::env::var("GOODSCANNER_HSR_LOCK_PROBE_EXPECT").unwrap();
    let result = AppendOnlyJsonJournalStore::new(PathBuf::from(path)).try_acquire_apply_lease();
    match expectation.as_str() {
        "busy" => {
            let error = result.expect_err("a different process already owns the mutation lease");
            assert_eq!(error.code(), "HSR_CONTROLLER_BUSY");
        },
        "available" => {
            let _lease = result.expect("the OS lease must be released when its owner exits/drops");
        },
        other => panic!("unexpected lock probe expectation: {other}"),
    }
}

#[test]
fn global_apply_lease_blocks_two_processes_with_different_journals_for_one_user_window() {
    let _serial = apply_lease_test_guard();
    let held_path = temporary_journal_path("cross-process-held");
    let competing_path = temporary_journal_path("cross-process-competing");
    let lease = AppendOnlyJsonJournalStore::new(&held_path)
        .try_acquire_apply_lease()
        .unwrap();
    let test_executable = std::env::current_exe().unwrap();
    let run_probe = |expectation: &str| {
        Command::new(&test_executable)
            .arg("--exact")
            .arg("journal_apply_lease_child_probe")
            .arg("--test-threads=1")
            .env("GOODSCANNER_HSR_LOCK_PROBE_PATH", &competing_path)
            .env("GOODSCANNER_HSR_LOCK_PROBE_EXPECT", expectation)
            .output()
            .unwrap()
    };

    // The paths intentionally differ: both processes still target the one HSR
    // client/window available to this user session, so the global lease wins.
    let blocked = run_probe("busy");
    assert!(
        blocked.status.success(),
        "cross-process contention probe failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&blocked.stdout),
        String::from_utf8_lossy(&blocked.stderr)
    );
    drop(lease);

    let released = run_probe("available");
    assert!(
        released.status.success(),
        "released-lock probe failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&released.stdout),
        String::from_utf8_lossy(&released.stderr)
    );
    remove_journal_and_lock(&held_path);
    remove_journal_and_lock(&competing_path);
}

#[test]
fn fresh_equipped_or_unknown_state_stops_without_clicking() {
    let envelope = envelope();
    let planning_observation = observed(
        envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let plan = build_manager_plan(&envelope, &[planning_observation]).unwrap();
    let authorization = ApplyAuthorization::new(&plan.digest, [MutationScope::Lock]);

    let equipped_matcher = VisibleGearMatcher {
        location_key: Some("1001".to_owned()),
        ..envelope.instructions[0].matcher.clone()
    };
    let mut equipped_device = SimulatedDevice::new(vec![observed(
        equipped_matcher,
        Some(false),
        Some(false),
        Some(true),
    )]);
    let mut equipped_store = MemoryJournalStore::default();
    let equipped_report = apply_manager_plan(
        &plan,
        &authorization,
        &mut equipped_device,
        &mut equipped_store,
    )
    .unwrap();
    assert_eq!(equipped_report.needs_review_actions, 1);
    assert!(equipped_device.toggles.is_empty());

    let mut unknown_device = SimulatedDevice::new(vec![observed(
        envelope.instructions[0].matcher.clone(),
        None,
        Some(false),
        Some(false),
    )]);
    let mut unknown_store = MemoryJournalStore::default();
    let unknown_report = apply_manager_plan(
        &plan,
        &authorization,
        &mut unknown_device,
        &mut unknown_store,
    )
    .unwrap();
    assert_eq!(unknown_report.needs_review_actions, 1);
    assert!(unknown_device.toggles.is_empty());
}

#[test]
fn two_action_batch_fail_stops_on_reread_failure_and_collateral_drift() {
    let envelope = two_instruction_envelope();
    let planning = two_before_observations(&envelope);
    let plan = build_manager_plan(&envelope, &planning).unwrap();
    let authorization = ApplyAuthorization::new(&plan.digest, [MutationScope::Lock]);

    let mut failed_device = SimulatedDevice::new(planning.clone());
    failed_device
        .reread_failures
        .insert(envelope.instructions[0].matcher.game_id);
    let mut failed_store = MemoryJournalStore::default();
    let failed =
        apply_manager_plan(&plan, &authorization, &mut failed_device, &mut failed_store).unwrap();
    assert_eq!(failed.needs_review_actions, 1);
    assert!(failed_device.toggles.is_empty());
    let failed_journal = failed_store.journal.as_ref().unwrap();
    assert_eq!(failed_journal.entries[0].status, JournalStatus::NeedsReview);
    assert_eq!(failed_journal.entries[1].status, JournalStatus::Pending);

    // The durable review marker prevents every still-pending action on restart.
    failed_device.reread_failures.clear();
    let saves_before_restart = failed_store.saves;
    let restarted =
        apply_manager_plan(&plan, &authorization, &mut failed_device, &mut failed_store).unwrap();
    assert_eq!(restarted.device_toggles, 0);
    assert_eq!(failed_store.saves, saves_before_restart);
    assert!(failed_device.toggles.is_empty());

    let mut drifted = planning;
    drifted[0].state.discard = Some(true);
    let mut drift_device = SimulatedDevice::new(drifted);
    let mut drift_store = MemoryJournalStore::default();
    let drift_report =
        apply_manager_plan(&plan, &authorization, &mut drift_device, &mut drift_store).unwrap();
    assert_eq!(drift_report.needs_review_actions, 1);
    assert!(drift_device.toggles.is_empty());
    let drift_journal = drift_store.journal.unwrap();
    assert_eq!(
        drift_journal.entries[0].outcome_code.as_deref(),
        Some("before_state_changed_or_unknown")
    );
    assert_eq!(drift_journal.entries[1].status, JournalStatus::Pending);
}

#[test]
fn two_action_batch_fail_stops_after_ambiguous_postverification() {
    let envelope = two_instruction_envelope();
    let planning = two_before_observations(&envelope);
    let plan = build_manager_plan(&envelope, &planning).unwrap();
    let authorization = ApplyAuthorization::new(&plan.digest, [MutationScope::Lock]);
    let mut device = SimulatedDevice::new(planning);
    device.duplicate_after_toggle = true;
    let mut store = MemoryJournalStore::default();

    let report = apply_manager_plan(&plan, &authorization, &mut device, &mut store).unwrap();
    assert_eq!(report.device_toggles, 1);
    assert_eq!(report.needs_review_actions, 1);
    assert_eq!(device.toggles, vec![MutationScope::Lock]);
    let journal = store.journal.unwrap();
    assert_eq!(journal.entries[0].status, JournalStatus::NeedsReview);
    assert_eq!(
        journal.entries[0].outcome_code.as_deref(),
        Some("postverification_failed")
    );
    assert_eq!(journal.entries[1].status, JournalStatus::Pending);
}

#[test]
fn lock_and_discard_are_distinct_authorization_scopes() {
    let lock_envelope = envelope();
    let observation = observed(
        lock_envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let lock_plan = build_manager_plan(&lock_envelope, std::slice::from_ref(&observation)).unwrap();
    assert_eq!(
        lock_plan.required_scopes(),
        BTreeSet::from([MutationScope::Lock])
    );

    let mut discard_envelope = lock_envelope;
    discard_envelope.instructions[0].desired.lock = None;
    discard_envelope.instructions[0].desired.discard = Some(true);
    refresh_envelope_idempotency(&mut discard_envelope);
    let discard_plan =
        build_manager_plan(&discard_envelope, std::slice::from_ref(&observation)).unwrap();
    assert_eq!(
        discard_plan.required_scopes(),
        BTreeSet::from([MutationScope::MarkDiscard])
    );

    let partial = ApplyAuthorization::new(&discard_plan.digest, [MutationScope::Lock]);
    let mut device = SimulatedDevice::new(vec![observation]);
    let mut store = MemoryJournalStore::default();
    assert!(apply_manager_plan(&discard_plan, &partial, &mut device, &mut store).is_err());
    assert!(device.toggles.is_empty());
}

#[test]
fn apply_refuses_mark_discard_when_fresh_relic_is_locked_without_toggling() {
    let _serial = apply_lease_test_guard();
    let mut discard_envelope = envelope();
    discard_envelope.instructions[0].desired.lock = None;
    discard_envelope.instructions[0].desired.discard = Some(true);
    refresh_envelope_idempotency(&mut discard_envelope);
    let planning = observed(
        discard_envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let plan = build_manager_plan(&discard_envelope, &[planning]).unwrap();
    let authorization = ApplyAuthorization::new(&plan.digest, [MutationScope::MarkDiscard]);
    let mut device = SimulatedDevice::new(vec![observed(
        discard_envelope.instructions[0].matcher.clone(),
        Some(true),
        Some(false),
        Some(false),
    )]);
    let path = temporary_journal_path("locked-discard-restart");

    let report = {
        let store = AppendOnlyJsonJournalStore::new(&path);
        let mut lease = store.try_acquire_apply_lease().unwrap();
        apply_manager_plan(&plan, &authorization, &mut device, &mut lease).unwrap()
    };

    assert_eq!(report.device_toggles, 0);
    assert_eq!(report.needs_review_actions, 1);
    assert_eq!(device.rereads, 1, "fresh lock state must be reread");
    assert!(device.toggles.is_empty());

    let mut reloaded = AppendOnlyJsonJournalStore::new(&path);
    let journal = reloaded
        .load()
        .expect("locked-discard refusal journal must remain reloadable")
        .expect("locked-discard refusal journal must exist");
    assert_eq!(
        journal.entries[0].outcome_code.as_deref(),
        Some("mark_discard_locked_or_lock_unknown")
    );

    let mut restarted_device = SimulatedDevice::new(Vec::new());
    let restarted = {
        let store = AppendOnlyJsonJournalStore::new(&path);
        let mut lease = store.try_acquire_apply_lease().unwrap();
        apply_manager_plan(&plan, &authorization, &mut restarted_device, &mut lease).unwrap()
    };
    assert_eq!(restarted.device_toggles, 0);
    assert_eq!(restarted.needs_review_actions, 1);
    assert_eq!(restarted_device.rereads, 0);
    assert!(restarted_device.toggles.is_empty());
    remove_journal_and_lock(&path);
}

#[test]
fn apply_rejects_malformed_mark_discard_plan_without_device_access() {
    let mut discard_envelope = envelope();
    discard_envelope.instructions[0].desired.lock = None;
    discard_envelope.instructions[0].desired.discard = Some(true);
    refresh_envelope_idempotency(&mut discard_envelope);
    let observation = observed(
        discard_envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let mut malformed =
        build_manager_plan(&discard_envelope, std::slice::from_ref(&observation)).unwrap();
    malformed.entries[0].observed_before.as_mut().unwrap().lock = Some(true);
    let authorization = ApplyAuthorization::new(&malformed.digest, [MutationScope::MarkDiscard]);
    let mut device = SimulatedDevice::new(vec![observation]);
    let mut store = MemoryJournalStore::default();

    let error = apply_manager_plan(&malformed, &authorization, &mut device, &mut store)
        .expect_err("locked before-evidence must invalidate mark-discard");

    assert_eq!(error.code(), "HSR_MANAGER_PLAN_EVIDENCE_INVALID");
    assert_eq!(device.rereads, 0);
    assert!(device.toggles.is_empty());
    assert!(store.journal.is_none());
}

#[test]
fn unlock_and_unmark_discard_require_their_own_scopes() {
    let mut unlock_envelope = envelope();
    unlock_envelope.instructions[0].before.lock = Some(true);
    unlock_envelope.instructions[0].before.discard = Some(true);
    unlock_envelope.instructions[0].desired.lock = Some(false);
    refresh_envelope_idempotency(&mut unlock_envelope);
    let observation = observed(
        unlock_envelope.instructions[0].matcher.clone(),
        Some(true),
        Some(true),
        Some(false),
    );
    let unlock_plan =
        build_manager_plan(&unlock_envelope, std::slice::from_ref(&observation)).unwrap();
    assert_eq!(
        unlock_plan.required_scopes(),
        BTreeSet::from([MutationScope::Unlock])
    );

    let mut unmark_envelope = unlock_envelope;
    unmark_envelope.instructions[0].before.discard = Some(true);
    unmark_envelope.instructions[0].desired.lock = None;
    unmark_envelope.instructions[0].desired.discard = Some(false);
    refresh_envelope_idempotency(&mut unmark_envelope);
    let unmark_plan = build_manager_plan(&unmark_envelope, &[observation]).unwrap();
    assert_eq!(
        unmark_plan.required_scopes(),
        BTreeSet::from([MutationScope::UnmarkDiscard])
    );
}

#[test]
fn manager_rejects_unsorted_or_duplicate_substat_keys() {
    let mut value: serde_json::Value = serde_json::from_str(GOLDEN_MANAGER_INSTRUCTIONS).unwrap();
    value["instructions"][0]["matcher"]["substats"] = serde_json::json!([
        {"key": "SpeedDelta", "value": 5.1},
        {"key": "CriticalChanceBase", "value": 8.7}
    ]);
    assert!(ManagerInstructionsEnvelope::parse_json(&value.to_string()).is_err());

    value["instructions"][0]["matcher"]["substats"] = serde_json::json!([
        {"key": "SpeedDelta", "value": 5.1},
        {"key": "SpeedDelta", "value": 8.0}
    ]);
    assert!(ManagerInstructionsEnvelope::parse_json(&value.to_string()).is_err());
}

#[test]
fn preview_and_apply_reports_have_english_and_chinese_user_text() {
    let envelope = envelope();
    let observation = observed(
        envelope.instructions[0].matcher.clone(),
        Some(false),
        Some(false),
        Some(false),
    );
    let plan = build_manager_plan(&envelope, std::slice::from_ref(&observation)).unwrap();
    let zh_preview = plan.render(Language::ZhCn);
    let en_preview = plan.render(Language::En);
    assert_eq!(zh_preview, GOLDEN_MANAGER_PREVIEW_ZH_CN.trim_end());
    assert_eq!(en_preview, GOLDEN_MANAGER_PREVIEW_EN.trim_end());
    assert_ne!(zh_preview, en_preview);

    let authorization = ApplyAuthorization::new(&plan.digest, [MutationScope::Lock]);
    let mut device = SimulatedDevice::new(vec![observation]);
    let mut store = MemoryJournalStore::default();
    let report = apply_manager_plan(&plan, &authorization, &mut device, &mut store).unwrap();
    assert!(report.render(Language::ZhCn).contains("已验证"));
    assert!(report.render(Language::En).contains("verified"));
}
