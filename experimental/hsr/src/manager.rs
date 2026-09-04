//! Experimental, user-confirmed HSR gear-state manager.
//!
//! This module deliberately contains no game-specific click coordinates.  It
//! plans mutations from a fresh visible inventory observation and executes
//! them only through the narrow [`ManagerMutationDevice`] boundary.  The
//! production adapter must independently enforce foreground/window checks.

use std::collections::{BTreeSet, HashSet};
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{HsrError, HsrResult};
use crate::localization::{Language, LocalizedText};
use crate::model::GearSlot;
use crate::reference::ReferenceCache;

pub const MANAGER_INSTRUCTIONS_SCHEMA: &str = "goodscanner.hsr.manager-instructions";
pub const MANAGER_INSTRUCTIONS_SCHEMA_VERSION: u32 = 1;
pub const MANAGER_JOURNAL_SCHEMA: &str = "goodscanner.hsr.manager-journal";
/// Version 2 persists the exact confirmed plan so a later process can safely
/// reconcile an interrupted or completed mutation without rebuilding it from
/// already-changed inventory state.
pub const MANAGER_JOURNAL_SCHEMA_VERSION: u32 = 2;

const INVALID_INSTRUCTIONS: LocalizedText = LocalizedText::new(
    "HSR 管理指令无效；不会执行任何游戏操作。",
    "The HSR manager instructions are invalid; no game action was performed.",
);
const CONFIRMATION_REQUIRED: LocalizedText = LocalizedText::new(
    "确认摘要或授权范围不匹配；不会执行任何游戏操作。",
    "The confirmation digest or authorization scopes do not match; no game action was performed.",
);
const JOURNAL_INVALID: LocalizedText = LocalizedText::new(
    "HSR 管理恢复日志无效；为安全起见已停止。",
    "The HSR manager recovery journal is invalid; execution stopped for safety.",
);
const JOURNAL_BUSY: LocalizedText = LocalizedText::new(
    "另一个 HSR 管理进程正在运行；不会执行任何游戏操作。",
    "Another HSR manager process is active; no game action was performed.",
);
const CONTROLLER_BUSY: LocalizedText = LocalizedText::new(
    "另一个实验性 HSR 操作正在控制本用户会话中的游戏；不会执行任何游戏操作。",
    "Another experimental HSR operation owns the game controller for this user session; no game action was performed.",
);
const CONTROLLER_UNAVAILABLE: LocalizedText = LocalizedText::new(
    "无法安全取得实验性 HSR 游戏控制器的独占锁；不会执行任何游戏操作。",
    "Could not safely acquire the exclusive experimental HSR game-controller lock; no game action was performed.",
);
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagerReference {
    pub schema_version: u32,
    pub provider: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagerPrivacy {
    pub account_identifiers_included: bool,
    pub raw_packet_data_included: bool,
    pub server_item_identifiers_included: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisibleStat {
    /// Canonical provider property key, for example `CriticalChanceBase`.
    /// Numeric stat IDs are intentionally not part of this contract.
    pub key: String,
    #[serde(serialize_with = "serialize_javascript_number")]
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisibleGearMatcher {
    pub key: String,
    /// Public gear-template ID, never an inventory/server item ID.
    pub game_id: u32,
    pub set_key: String,
    /// Public character key when known. `null` is not treated as proof that an
    /// item is unequipped; the fresh device observation decides that.
    pub location_key: Option<String>,
    pub rarity: u8,
    pub slot: GearSlot,
    pub level: u8,
    pub main_stat: VisibleStat,
    /// Must be sorted by canonical key, then value.
    pub substats: Vec<VisibleStat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedState {
    /// `null` means not observed and therefore cannot authorize a mutation.
    pub lock: Option<bool>,
    /// Reversible discard mark only; this is never salvage/delete/consume.
    pub discard: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesiredManagedState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discard: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagerInstruction {
    /// Correlation ID supplied by GGStarRail; it is not a game/server item ID.
    pub id: String,
    pub matcher: VisibleGearMatcher,
    pub before: ManagedState,
    pub desired: DesiredManagedState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagerInstructionsEnvelope {
    pub schema: String,
    pub schema_version: u32,
    pub request_id: String,
    /// GGStarRail-supplied deterministic key over its semantic instruction set
    /// and source inventory fingerprint. This scanner validates the wire shape
    /// and binds it into the preview/journal, but cannot reconstruct the source
    /// inventory fingerprint from this envelope.
    pub idempotency_key: String,
    pub reference: ManagerReference,
    pub privacy: ManagerPrivacy,
    pub instructions: Vec<ManagerInstruction>,
}

impl ManagerInstructionsEnvelope {
    /// Parse without echoing input bytes into errors or logs. Sensitive key
    /// names are rejected before typed deserialization.
    pub fn parse_json(input: &str) -> HsrResult<Self> {
        let value: Value = serde_json::from_str(input).map_err(|error| {
            HsrError::new(
                "HSR_MANAGER_JSON_INVALID",
                INVALID_INSTRUCTIONS,
                format!(
                    "manager JSON parse failed at line {} column {}",
                    error.line(),
                    error.column()
                ),
            )
        })?;
        reject_sensitive_fields(&value)?;
        let envelope: Self = serde_json::from_value(value).map_err(|error| {
            HsrError::new(
                "HSR_MANAGER_SCHEMA_INVALID",
                INVALID_INSTRUCTIONS,
                format!("manager schema validation failed ({:?})", error.classify()),
            )
        })?;
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn validate(&self) -> HsrResult<()> {
        if self.schema != MANAGER_INSTRUCTIONS_SCHEMA {
            return invalid("schema must be goodscanner.hsr.manager-instructions");
        }
        if self.schema_version != MANAGER_INSTRUCTIONS_SCHEMA_VERSION {
            return invalid("only manager instruction schemaVersion 1 is supported");
        }
        validate_request_id(&self.request_id)?;
        if self.reference.schema_version != 1 {
            return invalid("reference.schemaVersion must be 1");
        }
        if self.reference.provider != "gilore.ggstarrail-reference" {
            return invalid("reference.provider must be gilore.ggstarrail-reference");
        }
        validate_identifier("reference.revision", &self.reference.revision)?;
        if !is_sha256_key(&self.idempotency_key) {
            return Err(HsrError::new(
                "HSR_MANAGER_IDEMPOTENCY_KEY_INVALID",
                INVALID_INSTRUCTIONS,
                "idempotencyKey must use sha256 followed by 64 lowercase hexadecimal digits",
            ));
        }
        if self.privacy.account_identifiers_included
            || self.privacy.raw_packet_data_included
            || self.privacy.server_item_identifiers_included
        {
            return Err(HsrError::new(
                "HSR_MANAGER_PRIVACY_UNSAFE",
                INVALID_INSTRUCTIONS,
                "all privacy inclusion flags must be false",
            ));
        }

        let mut ids = HashSet::new();
        let mut matcher_fingerprints = HashSet::new();
        for instruction in &self.instructions {
            validate_stable_id("instruction.id", &instruction.id)?;
            if !ids.insert(instruction.id.as_str()) {
                return invalid("instruction IDs must be unique within one request");
            }
            validate_matcher(&instruction.matcher)?;
            let desired_count = usize::from(instruction.desired.lock.is_some())
                + usize::from(instruction.desired.discard.is_some());
            if desired_count == 0 {
                return invalid("each desired state must contain lock or discard");
            }
            if instruction.desired.lock == Some(true) && instruction.desired.discard == Some(true) {
                return invalid("lock=true and discard=true cannot be requested together");
            }
            if desired_count > 1 {
                return invalid(
                    "each instruction may request only one lock or discard state transition",
                );
            }
            let matcher_bytes = serde_json::to_vec(&instruction.matcher).map_err(|_| {
                HsrError::new(
                    "HSR_MANAGER_MATCHER_DIGEST_FAILED",
                    INVALID_INSTRUCTIONS,
                    "could not serialize a visible gear matcher",
                )
            })?;
            if !matcher_fingerprints.insert(matcher_bytes) {
                return invalid(
                    "a visible gear matcher may appear only once in one instruction envelope",
                );
            }
        }
        let expected_idempotency_key = self.expected_idempotency_key()?;
        if self.idempotency_key != expected_idempotency_key {
            return Err(HsrError::new(
                "HSR_MANAGER_IDEMPOTENCY_KEY_MISMATCH",
                INVALID_INSTRUCTIONS,
                "idempotencyKey does not match the semantic manager instruction payload",
            ));
        }
        Ok(())
    }

    /// Recompute the canonical semantic idempotency key. This is public so
    /// callers and cross-contract tests can construct a modified typed fixture
    /// without duplicating the canonical ordering/hash implementation.
    pub fn expected_idempotency_key(&self) -> HsrResult<String> {
        compute_instruction_idempotency_key(&self.reference.revision, &self.instructions)
    }
}

/// Validate the complete manager envelope against the exact normalized
/// reference cache that will be used for the fresh device scan. Callers should
/// run this before constructing a device/controller so reference drift cannot
/// reach the input boundary.
pub fn validate_manager_envelope_reference(
    envelope: &ManagerInstructionsEnvelope,
    reference: &ReferenceCache,
) -> HsrResult<()> {
    envelope.validate()?;
    if envelope.reference.schema_version != reference.schema_version()
        || envelope.reference.provider != reference.provider()
        || envelope.reference.revision != reference.revision()
    {
        return manager_reference_mismatch(
            "manager reference identity does not equal the loaded reference cache",
        );
    }

    for instruction in &envelope.instructions {
        let matcher = &instruction.matcher;
        let gear = reference.gear(matcher.game_id).ok_or_else(|| {
            manager_reference_error(format!(
                "instruction {} uses an unknown public gear gameId",
                instruction.id
            ))
        })?;
        if gear.key != matcher.key
            || gear.set_key != matcher.set_key
            || gear.rarity != matcher.rarity
            || gear.slot != matcher.slot
            || matcher.level > gear.max_level
        {
            return manager_reference_mismatch(format!(
                "instruction {} gear matcher does not equal the loaded public reference",
                instruction.id
            ));
        }

        let canonical = reference
            .canonical_gear_for_observation(
                matcher.game_id,
                &matcher.main_stat.key,
                matcher.level,
                matcher.main_stat.value,
            )
            .ok_or_else(|| {
                manager_reference_error(format!(
                    "instruction {} gear matcher is not a canonical visible-equivalence reference",
                    instruction.id
                ))
            })?;
        if canonical.game_id != matcher.game_id {
            return manager_reference_mismatch(format!(
                "instruction {} uses a non-canonical public gear gameId",
                instruction.id
            ));
        }

        let expected_main_value = reference
            .relic_main_stat_value_for_piece(gear, &matcher.main_stat.key, matcher.level)
            .ok_or_else(|| {
                manager_reference_error(format!(
                    "instruction {} main stat cannot be resolved from the loaded progression reference",
                    instruction.id
                ))
            })?;
        if (expected_main_value - matcher.main_stat.value).abs() > 1e-6 {
            return manager_reference_mismatch(format!(
                "instruction {} main-stat value does not equal the loaded progression reference",
                instruction.id
            ));
        }
        if matcher
            .substats
            .iter()
            .any(|stat| reference.stat(&stat.key).is_none())
        {
            return manager_reference_mismatch(format!(
                "instruction {} contains an unknown public substat key",
                instruction.id
            ));
        }
        if let Some(location_key) = matcher.location_key.as_ref() {
            let location_id = location_key.parse::<u32>().map_err(|_| {
                manager_reference_error(format!(
                    "instruction {} locationKey is not resolvable as a public character ID",
                    instruction.id
                ))
            })?;
            let location = reference.character(location_id).ok_or_else(|| {
                manager_reference_error(format!(
                    "instruction {} locationKey is absent from the loaded reference",
                    instruction.id
                ))
            })?;
            if location.key != *location_key {
                return manager_reference_mismatch(format!(
                    "instruction {} locationKey does not equal the loaded public reference",
                    instruction.id
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedGearObservation {
    pub matcher: VisibleGearMatcher,
    pub state: ManagedState,
    /// `Some(false)` is the only state allowed to mutate. `Some(true)` and
    /// `None` (not observed) are preview-only.
    pub equipped: Option<bool>,
}

impl ManagedGearObservation {
    pub fn validate(&self) -> HsrResult<()> {
        validate_matcher(&self.matcher)?;
        match (self.equipped, self.matcher.location_key.as_ref()) {
            (Some(false), Some(_)) => invalid("unequipped observation cannot have locationKey"),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanClassification {
    Actionable,
    NoOpAlreadyDesired,
    PreviewOnlyNotFound,
    PreviewOnlyAmbiguous,
    PreviewOnlyEquipped,
    PreviewOnlyEquipmentUnknown,
    PreviewOnlyLocked,
    PreviewOnlyUnknownBefore,
    PreviewOnlyBeforeMismatch,
}

impl PlanClassification {
    fn label(self, language: Language) -> &'static str {
        match (self, language) {
            (Self::Actionable, Language::ZhCn) => "可执行",
            (Self::Actionable, Language::En) => "actionable",
            (Self::NoOpAlreadyDesired, Language::ZhCn) => "已是目标状态，无需操作",
            (Self::NoOpAlreadyDesired, Language::En) => "already in the desired state; no action",
            (Self::PreviewOnlyNotFound, Language::ZhCn) => "仅预览：未找到匹配装备",
            (Self::PreviewOnlyNotFound, Language::En) => "preview only: matching gear not found",
            (Self::PreviewOnlyAmbiguous, Language::ZhCn) => "仅预览：匹配到多个装备",
            (Self::PreviewOnlyAmbiguous, Language::En) => "preview only: multiple gear matches",
            (Self::PreviewOnlyEquipped, Language::ZhCn) => "仅预览：装备正在使用中",
            (Self::PreviewOnlyEquipped, Language::En) => "preview only: gear is equipped",
            (Self::PreviewOnlyEquipmentUnknown, Language::ZhCn) => {
                "仅预览：无法确认装备是否正在使用"
            },
            (Self::PreviewOnlyEquipmentUnknown, Language::En) => {
                "preview only: equipped state is unknown"
            },
            (Self::PreviewOnlyLocked, Language::ZhCn) => "仅预览：装备已锁定",
            (Self::PreviewOnlyLocked, Language::En) => "preview only: gear is locked",
            (Self::PreviewOnlyUnknownBefore, Language::ZhCn) => "仅预览：变更前状态未知",
            (Self::PreviewOnlyUnknownBefore, Language::En) => {
                "preview only: before-state is unknown"
            },
            (Self::PreviewOnlyBeforeMismatch, Language::ZhCn) => "仅预览：当前状态与指令不一致",
            (Self::PreviewOnlyBeforeMismatch, Language::En) => {
                "preview only: current state differs from the instruction"
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ManagedField {
    Lock,
    Discard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MutationScope {
    Lock,
    Unlock,
    MarkDiscard,
    UnmarkDiscard,
}

impl MutationScope {
    pub fn from_change(field: ManagedField, desired: bool) -> Self {
        match (field, desired) {
            (ManagedField::Lock, true) => Self::Lock,
            (ManagedField::Lock, false) => Self::Unlock,
            (ManagedField::Discard, true) => Self::MarkDiscard,
            (ManagedField::Discard, false) => Self::UnmarkDiscard,
        }
    }

    pub(crate) fn has_required_lock_evidence(self, state: Option<&ManagedState>) -> bool {
        self != Self::MarkDiscard || state.and_then(|state| state.lock) == Some(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExactChange {
    pub field: ManagedField,
    pub before: bool,
    pub desired: bool,
    pub scope: MutationScope,
}

/// The single transition requested by one instruction, retained even when
/// the fresh evidence makes the instruction preview-only. This lets the user
/// confirm exactly what was requested without turning it into authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestedChange {
    pub field: ManagedField,
    pub declared_before: Option<bool>,
    pub desired: bool,
    pub scope: MutationScope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagerPlanEntry {
    pub instruction_id: String,
    pub matcher: VisibleGearMatcher,
    pub requested_change: RequestedChange,
    pub classification: PlanClassification,
    pub observed_before: Option<ManagedState>,
    /// `Some(false)` is positive proof that the matched item was unequipped.
    /// `Some(true)` proves it was equipped; `None` is explicitly not proof.
    pub observed_equipped: Option<bool>,
    pub changes: Vec<ExactChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagerPlan {
    pub request_id: String,
    pub idempotency_key: String,
    pub reference: ManagerReference,
    pub inventory_fingerprint: String,
    pub digest: String,
    pub entries: Vec<ManagerPlanEntry>,
}

impl ManagerPlan {
    pub fn required_scopes(&self) -> BTreeSet<MutationScope> {
        self.entries
            .iter()
            .flat_map(|entry| entry.changes.iter().map(|change| change.scope))
            .collect()
    }

    pub fn render(&self, language: Language) -> String {
        let actionable = self
            .entries
            .iter()
            .filter(|entry| entry.classification == PlanClassification::Actionable)
            .count();
        let heading = match language {
            Language::ZhCn => format!(
                "HSR 实验性管理预览：{} 条指令，{} 条可执行。\n请求 ID：{}\n参考：{} @ {}\n库存指纹：{}\n确认摘要：{}",
                self.entries.len(),
                actionable,
                preview_json(&self.request_id),
                preview_json(&self.reference.provider),
                preview_json(&self.reference.revision),
                self.inventory_fingerprint,
                self.digest
            ),
            Language::En => format!(
                "Experimental HSR manager preview: {} instruction(s), {} actionable.\nRequest ID: {}\nReference: {} @ {}\nInventory fingerprint: {}\nConfirmation digest: {}",
                self.entries.len(),
                actionable,
                preview_json(&self.request_id),
                preview_json(&self.reference.provider),
                preview_json(&self.reference.revision),
                self.inventory_fingerprint,
                self.digest
            ),
        };
        let details = self
            .entries
            .iter()
            .map(|entry| {
                let matcher = preview_json(&entry.matcher);
                let observed = preview_json(&entry.observed_before);
                let equipped = preview_json(&entry.observed_equipped);
                let requested = preview_json(&entry.requested_change);
                let authorized = preview_json(&entry.changes);
                match language {
                    Language::ZhCn => format!(
                        "- 指令 {}\n  分类：{}\n  完整可见匹配器：{}\n  已观察状态（锁定与弃置标记）：{}\n  装备状态证明：{}\n  请求的精确变更与范围：{}\n  可授权变更：{}",
                        preview_json(&entry.instruction_id),
                        entry.classification.label(language),
                        matcher,
                        observed,
                        equipped,
                        requested,
                        authorized
                    ),
                    Language::En => format!(
                        "- Instruction {}\n  Classification: {}\n  Complete visible matcher: {}\n  Observed state (lock and discard): {}\n  Equipped-state proof: {}\n  Exact requested transition and scope: {}\n  Authorizable change(s): {}",
                        preview_json(&entry.instruction_id),
                        entry.classification.label(language),
                        matcher,
                        observed,
                        equipped,
                        requested,
                        authorized
                    ),
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        if details.is_empty() {
            heading
        } else {
            format!("{heading}\n{details}")
        }
    }
}

/// Match instructions against a complete, freshly scanned gear inventory.
pub fn build_manager_plan(
    envelope: &ManagerInstructionsEnvelope,
    fresh_inventory: &[ManagedGearObservation],
) -> HsrResult<ManagerPlan> {
    envelope.validate()?;
    for observation in fresh_inventory {
        observation.validate()?;
    }

    let inventory_fingerprint = compute_inventory_fingerprint(fresh_inventory)?;
    let entries = envelope
        .instructions
        .iter()
        .map(|instruction| plan_instruction(instruction, fresh_inventory))
        .collect::<Vec<_>>();

    let digest = compute_plan_digest(
        &envelope.request_id,
        &envelope.idempotency_key,
        &envelope.reference,
        &inventory_fingerprint,
        &entries,
    )?;

    Ok(ManagerPlan {
        request_id: envelope.request_id.clone(),
        idempotency_key: envelope.idempotency_key.clone(),
        reference: envelope.reference.clone(),
        inventory_fingerprint,
        digest,
        entries,
    })
}

fn plan_instruction(
    instruction: &ManagerInstruction,
    inventory: &[ManagedGearObservation],
) -> ManagerPlanEntry {
    let candidates = inventory
        .iter()
        .filter(|observation| matcher_matches(&instruction.matcher, &observation.matcher))
        .collect::<Vec<_>>();

    let requested_change = requested_change(instruction);
    let (classification, observed_before, observed_equipped, changes) = match candidates.as_slice()
    {
        [] => (
            PlanClassification::PreviewOnlyNotFound,
            None,
            None,
            Vec::new(),
        ),
        [observation] if observation.equipped == Some(true) => (
            PlanClassification::PreviewOnlyEquipped,
            Some(observation.state),
            observation.equipped,
            Vec::new(),
        ),
        [observation] if observation.equipped.is_none() => (
            PlanClassification::PreviewOnlyEquipmentUnknown,
            Some(observation.state),
            observation.equipped,
            Vec::new(),
        ),
        [observation] => {
            let (classification, observed_before, changes) =
                classify_state(instruction, observation);
            (
                classification,
                observed_before,
                observation.equipped,
                changes,
            )
        },
        _ => (
            PlanClassification::PreviewOnlyAmbiguous,
            None,
            None,
            Vec::new(),
        ),
    };

    ManagerPlanEntry {
        instruction_id: instruction.id.clone(),
        matcher: instruction.matcher.clone(),
        requested_change,
        classification,
        observed_before,
        observed_equipped,
        changes,
    }
}

fn requested_change(instruction: &ManagerInstruction) -> RequestedChange {
    let (field, declared_before, desired) =
        match (instruction.desired.lock, instruction.desired.discard) {
            (Some(desired), None) => (ManagedField::Lock, instruction.before.lock, desired),
            (None, Some(desired)) => (ManagedField::Discard, instruction.before.discard, desired),
            _ => unreachable!("validated manager instruction must request exactly one field"),
        };
    RequestedChange {
        field,
        declared_before,
        desired,
        scope: MutationScope::from_change(field, desired),
    }
}

fn classify_state(
    instruction: &ManagerInstruction,
    observation: &ManagedGearObservation,
) -> (PlanClassification, Option<ManagedState>, Vec<ExactChange>) {
    // Both managed flags are required for every actionable instruction. The
    // non-requested flag is collateral-state evidence: drift there is just as
    // important as drift in the field that would be clicked.
    let (Some(declared_lock), Some(declared_discard), Some(current_lock), Some(current_discard)) = (
        instruction.before.lock,
        instruction.before.discard,
        observation.state.lock,
        observation.state.discard,
    ) else {
        return (
            PlanClassification::PreviewOnlyUnknownBefore,
            Some(observation.state),
            Vec::new(),
        );
    };

    let requested = requested_change(instruction);
    let current_requested = match requested.field {
        ManagedField::Lock => current_lock,
        ManagedField::Discard => current_discard,
    };

    // HSR requires unlocking before a relic can be marked for discard. Keep
    // those as two separately previewed and confirmed runs; never turn one
    // instruction into an implicit multi-step mutation.
    if requested.scope == MutationScope::MarkDiscard && (declared_lock || current_lock) {
        return (
            PlanClassification::PreviewOnlyLocked,
            Some(observation.state),
            Vec::new(),
        );
    }

    // A completed state is a safe no-op even when this envelope is replayed.
    // All four before-state values were still required above so the preview is
    // complete and cannot silently hide collateral uncertainty.
    if current_requested == requested.desired {
        return (
            PlanClassification::NoOpAlreadyDesired,
            Some(observation.state),
            Vec::new(),
        );
    }

    if current_lock != declared_lock || current_discard != declared_discard {
        return (
            PlanClassification::PreviewOnlyBeforeMismatch,
            Some(observation.state),
            Vec::new(),
        );
    }

    (
        PlanClassification::Actionable,
        Some(observation.state),
        vec![ExactChange {
            field: requested.field,
            before: current_requested,
            desired: requested.desired,
            scope: requested.scope,
        }],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyAuthorization {
    pub confirmed_plan_digest: String,
    pub scopes: BTreeSet<MutationScope>,
}

impl ApplyAuthorization {
    pub fn new(
        confirmed_plan_digest: impl Into<String>,
        scopes: impl IntoIterator<Item = MutationScope>,
    ) -> Self {
        Self {
            confirmed_plan_digest: confirmed_plan_digest.into(),
            scopes: scopes.into_iter().collect(),
        }
    }
}

/// Device implementation for the experimental manager. Implementations must
/// select by the complete visible matcher and perform exactly one UI toggle.
pub trait ManagerMutationDevice {
    fn reread(
        &mut self,
        matcher: &VisibleGearMatcher,
    ) -> Result<Vec<ManagedGearObservation>, String>;

    fn toggle_once(
        &mut self,
        target: &ManagedGearObservation,
        scope: MutationScope,
    ) -> Result<(), String>;
}

pub trait ManagerJournalStore {
    fn load(&mut self) -> Result<Option<ManagerJournal>, String>;
    fn save(&mut self, journal: &ManagerJournal) -> Result<(), String>;

    /// True only while this store owns a cross-process exclusive apply lease.
    /// Mutation entry points reject stores that do not provide this guarantee.
    fn holds_exclusive_apply_lease(&self) -> bool;
}

/// Append-only JSON journal. Each complete line is one durable snapshot. If a
/// process stops during a write, loading falls back to the preceding complete
/// line, preserving the pre-click `mutationStarted` state.
#[derive(Debug, Clone)]
pub struct AppendOnlyJsonJournalStore {
    path: PathBuf,
}

/// Cross-process, per-user-session lease for every command that can drive the
/// HSR game window. The legacy lock filename remains stable so an older
/// experimental manager process still excludes a newer scan or preview.
#[derive(Debug)]
pub struct HsrControllerLease {
    _lock_file: File,
}

impl HsrControllerLease {
    pub fn try_acquire() -> HsrResult<Self> {
        let lock_path = global_hsr_controller_lock_path();
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| {
                HsrError::new(
                    "HSR_CONTROLLER_LOCK_IO",
                    CONTROLLER_UNAVAILABLE,
                    format!("global HSR controller lock open failed: {error}"),
                )
            })?;
        lock_file.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => HsrError::new(
                "HSR_CONTROLLER_BUSY",
                CONTROLLER_BUSY,
                "another process owns the per-user HSR game-controller lease",
            ),
            TryLockError::Error(error) => HsrError::new(
                "HSR_CONTROLLER_LOCK_IO",
                CONTROLLER_UNAVAILABLE,
                format!("global HSR controller lock acquisition failed: {error}"),
            ),
        })?;
        Ok(Self {
            _lock_file: lock_file,
        })
    }
}

impl AppendOnlyJsonJournalStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Acquire a non-blocking kernel-backed exclusive lease for this journal.
    /// The lease is released automatically on drop, including process exit.
    pub fn try_acquire_apply_lease(self) -> HsrResult<AppendOnlyJsonJournalLease> {
        self.try_acquire_apply_lease_with_controller(HsrControllerLease::try_acquire()?)
    }

    /// Bind an already-held universal controller lease to this journal lease.
    /// Production CLI callers acquire the controller first so contention is
    /// rejected before reference loading or device construction.
    pub fn try_acquire_apply_lease_with_controller(
        self,
        controller_lease: HsrControllerLease,
    ) -> HsrResult<AppendOnlyJsonJournalLease> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                HsrError::new(
                    "HSR_MANAGER_JOURNAL_LOCK_IO",
                    JOURNAL_INVALID,
                    format!("journal lock directory create failed: {error}"),
                )
            })?;
        }
        let journal_lock_path = journal_lock_path(&self.path);
        let journal_lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&journal_lock_path)
            .map_err(|error| {
                HsrError::new(
                    "HSR_MANAGER_JOURNAL_LOCK_IO",
                    JOURNAL_INVALID,
                    format!("journal lock open failed: {error}"),
                )
            })?;
        journal_lock_file.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => HsrError::new(
                "HSR_MANAGER_JOURNAL_BUSY",
                JOURNAL_BUSY,
                "exclusive manager journal lease is already held",
            ),
            TryLockError::Error(error) => HsrError::new(
                "HSR_MANAGER_JOURNAL_LOCK_IO",
                JOURNAL_INVALID,
                format!("journal lock acquisition failed: {error}"),
            ),
        })?;
        Ok(AppendOnlyJsonJournalLease {
            store: self,
            _controller_lease: controller_lease,
            _journal_lock_file: journal_lock_file,
        })
    }

    fn repair_unterminated_tail(&mut self) -> Result<(), String> {
        let contents = match std::fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("journal repair read failed: {error}")),
        };
        if contents.is_empty() || contents.ends_with('\n') {
            return Ok(());
        }

        // Validate every durable line and ensure the unterminated suffix is
        // either a complete-but-unsynced snapshot or a genuine EOF truncation.
        // Syntax/data corruption must never be silently removed.
        self.load()?;
        let verified_len = contents.rfind('\n').map_or(0, |index| index + 1);
        let file = OpenOptions::new()
            .write(true)
            .open(&self.path)
            .map_err(|error| format!("journal repair open failed: {error}"))?;
        file.set_len(verified_len as u64)
            .map_err(|error| format!("journal repair truncate failed: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("journal repair durability sync failed: {error}"))
    }
}

fn journal_lock_path(path: &Path) -> PathBuf {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".apply.lock");
    PathBuf::from(lock_path)
}

fn global_hsr_controller_lock_path() -> PathBuf {
    std::env::temp_dir().join("goodscanner-hsr-manager-mutation-v1.lock")
}

/// Exclusive append-only journal ownership held across reread, click, and
/// durable terminal write. Dropping the file handle releases the OS lock.
#[derive(Debug)]
pub struct AppendOnlyJsonJournalLease {
    store: AppendOnlyJsonJournalStore,
    _controller_lease: HsrControllerLease,
    _journal_lock_file: File,
}

impl AppendOnlyJsonJournalLease {
    pub fn path(&self) -> &Path {
        self.store.path()
    }
}

impl ManagerJournalStore for AppendOnlyJsonJournalLease {
    fn load(&mut self) -> Result<Option<ManagerJournal>, String> {
        self.store.load()
    }

    fn save(&mut self, journal: &ManagerJournal) -> Result<(), String> {
        self.store.repair_unterminated_tail()?;
        self.store.save(journal)
    }

    fn holds_exclusive_apply_lease(&self) -> bool {
        true
    }
}

impl ManagerJournalStore for AppendOnlyJsonJournalStore {
    fn load(&mut self) -> Result<Option<ManagerJournal>, String> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("journal open failed: {error}")),
        };
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|error| format!("journal read failed: {error}"))?;

        let lines = contents.lines().collect::<Vec<_>>();
        let ends_with_complete_line = contents.ends_with('\n');
        let mut last = None;
        for (index, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ManagerJournal>(line) {
                Ok(journal) => {
                    validate_journal_snapshot_for_store(&journal)?;
                    if let Some(previous) = last.as_ref() {
                        validate_journal_transition_for_store(previous, &journal)?;
                    } else if journal.entries.iter().any(|entry| {
                        entry.status != JournalStatus::Pending
                            || entry.toggle_attempts != 0
                            || entry.outcome_code.is_some()
                    }) {
                        return Err("journal does not begin with a pending snapshot".to_owned());
                    }
                    last = Some(journal);
                },
                Err(error)
                    if index + 1 == lines.len() && !ends_with_complete_line && error.is_eof() =>
                {
                    break;
                },
                Err(_) => return Err(format!("journal line {} is invalid", index + 1)),
            }
        }
        Ok(last)
    }

    fn save(&mut self, journal: &ManagerJournal) -> Result<(), String> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("journal directory create failed: {error}"))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| format!("journal append open failed: {error}"))?;
        serde_json::to_writer(&mut file, journal)
            .map_err(|error| format!("journal serialization failed: {error}"))?;
        file.write_all(b"\n")
            .map_err(|error| format!("journal append failed: {error}"))?;
        file.flush()
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("journal durability sync failed: {error}"))
    }

    fn holds_exclusive_apply_lease(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JournalStatus {
    Pending,
    MutationStarted,
    Verified,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JournalEntry {
    pub instruction_id: String,
    pub change: ExactChange,
    pub status: JournalStatus,
    pub toggle_attempts: u8,
    pub outcome_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagerJournal {
    pub schema: String,
    pub schema_version: u32,
    pub request_id: String,
    pub idempotency_key: String,
    pub plan_digest: String,
    pub plan: ManagerPlan,
    pub entries: Vec<JournalEntry>,
}

impl ManagerJournal {
    fn for_plan(plan: &ManagerPlan) -> Self {
        let entries = plan
            .entries
            .iter()
            .filter(|entry| entry.classification == PlanClassification::Actionable)
            .flat_map(|entry| {
                entry.changes.iter().cloned().map(|change| JournalEntry {
                    instruction_id: entry.instruction_id.clone(),
                    change,
                    status: JournalStatus::Pending,
                    toggle_attempts: 0,
                    outcome_code: None,
                })
            })
            .collect();
        Self {
            schema: MANAGER_JOURNAL_SCHEMA.to_owned(),
            schema_version: MANAGER_JOURNAL_SCHEMA_VERSION,
            request_id: plan.request_id.clone(),
            idempotency_key: plan.idempotency_key.clone(),
            plan_digest: plan.digest.clone(),
            plan: plan.clone(),
            entries,
        }
    }

    fn validate_for_plan(&self, plan: &ManagerPlan) -> HsrResult<()> {
        let expected = Self::for_plan(plan);
        if self.schema != expected.schema
            || self.schema_version != expected.schema_version
            || self.request_id != expected.request_id
            || self.idempotency_key != expected.idempotency_key
            || self.plan_digest != expected.plan_digest
            || self.plan != expected.plan
            || self.entries.len() != expected.entries.len()
        {
            return Err(HsrError::new(
                "HSR_MANAGER_JOURNAL_MISMATCH",
                JOURNAL_INVALID,
                "journal identity or action count does not match the confirmed plan",
            ));
        }
        for (actual, expected) in self.entries.iter().zip(expected.entries.iter()) {
            if actual.instruction_id != expected.instruction_id || actual.change != expected.change
            {
                return Err(HsrError::new(
                    "HSR_MANAGER_JOURNAL_MISMATCH",
                    JOURNAL_INVALID,
                    "journal actions do not match the confirmed plan",
                ));
            }
            if !valid_journal_entry_state(actual) {
                return Err(HsrError::new(
                    "HSR_MANAGER_JOURNAL_STATUS_INVALID",
                    JOURNAL_INVALID,
                    "journal status is inconsistent with its toggle-attempt count or outcome code",
                ));
            }
        }
        Ok(())
    }
}

fn valid_journal_entry_state(entry: &JournalEntry) -> bool {
    matches!(
        (
            entry.status,
            entry.toggle_attempts,
            entry.outcome_code.as_deref(),
        ),
        (JournalStatus::Pending, 0, None)
            | (JournalStatus::MutationStarted, 1, None)
            | (
                JournalStatus::Verified,
                0,
                Some("already_desired_on_reread")
            )
            | (
                JournalStatus::Verified,
                1,
                Some(
                    "recovered_desired_state" | "postverified" | "postverified_after_device_error"
                ),
            )
            | (
                JournalStatus::NeedsReview,
                0,
                Some(
                    "device_reread_failed"
                        | "fresh_match_not_found"
                        | "fresh_match_ambiguous"
                        | "fresh_observation_invalid"
                        | "fresh_item_equipped"
                        | "fresh_equipped_state_unknown"
                        | "mark_discard_locked_or_lock_unknown"
                        | "before_state_changed_or_unknown",
                ),
            )
            | (
                JournalStatus::NeedsReview,
                1,
                Some(
                    "interrupted_mutation_ambiguous"
                        | "postverification_failed"
                        | "device_error_state_ambiguous",
                ),
            )
    )
}

fn validate_journal_snapshot_for_store(journal: &ManagerJournal) -> Result<(), String> {
    if journal.schema != MANAGER_JOURNAL_SCHEMA
        || journal.schema_version != MANAGER_JOURNAL_SCHEMA_VERSION
        || journal
            .entries
            .iter()
            .any(|entry| !valid_journal_entry_state(entry))
    {
        return Err("journal snapshot has an invalid schema or entry state".to_owned());
    }
    validate_plan_digest(&journal.plan)
        .and_then(|_| journal.validate_for_plan(&journal.plan))
        .map_err(|_| "journal snapshot contains an invalid persisted plan".to_owned())?;
    Ok(())
}

fn validate_journal_transition_for_store(
    previous: &ManagerJournal,
    next: &ManagerJournal,
) -> Result<(), String> {
    if previous.schema != next.schema
        || previous.schema_version != next.schema_version
        || previous.request_id != next.request_id
        || previous.idempotency_key != next.idempotency_key
        || previous.plan_digest != next.plan_digest
        || previous.entries.len() != next.entries.len()
    {
        return Err("journal identity changed between append-only snapshots".to_owned());
    }

    for (previous_entry, next_entry) in previous.entries.iter().zip(next.entries.iter()) {
        if previous_entry.instruction_id != next_entry.instruction_id
            || previous_entry.change != next_entry.change
        {
            return Err("journal action changed between append-only snapshots".to_owned());
        }
        let unchanged = previous_entry == next_entry;
        let forward = match (previous_entry.status, next_entry.status) {
            (JournalStatus::Pending, JournalStatus::MutationStarted) => {
                next_entry.toggle_attempts == 1 && next_entry.outcome_code.is_none()
            },
            (JournalStatus::Pending, JournalStatus::Verified) => {
                next_entry.toggle_attempts == 0
                    && next_entry.outcome_code.as_deref() == Some("already_desired_on_reread")
            },
            (JournalStatus::Pending, JournalStatus::NeedsReview) => next_entry.toggle_attempts == 0,
            (JournalStatus::MutationStarted, JournalStatus::Verified)
            | (JournalStatus::MutationStarted, JournalStatus::NeedsReview) => {
                next_entry.toggle_attempts == 1
            },
            _ => false,
        };
        if !unchanged && !forward {
            return Err("journal entry moved backward or skipped a durable state".to_owned());
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    /// Correlation ID from the exact plan that first initialized the journal.
    pub original_request_id: String,
    /// Correlation ID on this invocation. It may differ on a semantic retry
    /// because requestId is intentionally excluded from idempotency.
    pub submitted_request_id: String,
    pub total_actions: usize,
    pub verified_actions: usize,
    pub needs_review_actions: usize,
    pub device_toggles: usize,
}

impl ApplyReport {
    pub fn render(&self, language: Language) -> String {
        match language {
            Language::ZhCn => format!(
                "HSR 实验性管理结果：原请求 {}，本次请求 {}；共 {} 项变更，{} 项已验证，{} 项需人工复核；设备切换 {} 次。",
                preview_json(&self.original_request_id),
                preview_json(&self.submitted_request_id),
                self.total_actions,
                self.verified_actions,
                self.needs_review_actions,
                self.device_toggles
            ),
            Language::En => format!(
                "Experimental HSR manager result: original request {}, submitted request {}; {} change(s), {} verified, {} need review; {} device toggle(s).",
                preview_json(&self.original_request_id),
                preview_json(&self.submitted_request_id),
                self.total_actions,
                self.verified_actions,
                self.needs_review_actions,
                self.device_toggles
            ),
        }
    }
}

/// Load the exact original plan retained by an existing manager journal.
///
/// This is the read-only recovery seam for attended callers: it validates the
/// current instruction envelope, maps journal I/O into the manager error
/// contract, and rejects any persisted plan whose digest, reference, or
/// semantic instructions no longer match. It does not acquire a controller or
/// perform any device action.
pub fn load_manager_recovery_plan<S: ManagerJournalStore>(
    envelope: &ManagerInstructionsEnvelope,
    journal_store: &mut S,
) -> HsrResult<Option<ManagerPlan>> {
    envelope.validate()?;
    let Some(journal) = journal_store.load().map_err(journal_io_error)? else {
        return Ok(None);
    };
    validate_persisted_plan_for_envelope(&journal, envelope)?;
    Ok(Some(journal.plan))
}

/// Journal-first manager entry point used by attended CLI apply. An existing
/// journal supplies its original exact plan before `fresh_inventory` is ever
/// invoked, so completed retries are idempotent and `mutationStarted` retries
/// can reconcile the one prior click against the original confirmation.
pub fn apply_manager_envelope<D, S, F, P>(
    envelope: &ManagerInstructionsEnvelope,
    authorization: &ApplyAuthorization,
    device: &mut D,
    journal_store: &mut S,
    fresh_inventory: F,
    review_exact_plan: P,
) -> HsrResult<ApplyReport>
where
    D: ManagerMutationDevice,
    S: ManagerJournalStore,
    F: FnOnce(&mut D) -> HsrResult<Vec<ManagedGearObservation>>,
    P: FnOnce(&ManagerPlan) -> HsrResult<()>,
{
    require_exclusive_apply_lease(journal_store)?;
    let plan = match load_manager_recovery_plan(envelope, journal_store)? {
        Some(plan) => plan,
        None => {
            let inventory = fresh_inventory(device)?;
            build_manager_plan(envelope, &inventory)?
        },
    };

    // Re-display the exact fresh or recovered plan before authorization can
    // reach any mutation. CLI callers use this hook for the bilingual text and
    // exact JSON preview; a rendering failure therefore remains fail-closed.
    review_exact_plan(&plan)?;
    let mut report = apply_manager_plan(&plan, authorization, device, journal_store)?;
    report.submitted_request_id = envelope.request_id.clone();
    Ok(report)
}

/// Apply a confirmed plan. A journal snapshot is durably saved immediately
/// before every toggle and after verification. A recovered `mutationStarted`
/// action is reread but never blindly toggled again.
pub fn apply_manager_plan<D: ManagerMutationDevice, S: ManagerJournalStore>(
    plan: &ManagerPlan,
    authorization: &ApplyAuthorization,
    device: &mut D,
    journal_store: &mut S,
) -> HsrResult<ApplyReport> {
    require_exclusive_apply_lease(journal_store)?;
    validate_plan_action_safety(plan)?;
    validate_plan_digest(plan)?;
    authorize(plan, authorization)?;

    let mut journal = match journal_store.load().map_err(journal_io_error)? {
        Some(journal) => {
            journal.validate_for_plan(plan)?;
            journal
        },
        None => {
            let journal = ManagerJournal::for_plan(plan);
            save_and_reload_exact(journal_store, &journal)?;
            journal
        },
    };

    // A prior unresolved action is a batch-wide stop marker. In particular,
    // never let a pending entry earlier in the vector run merely because the
    // NeedsReview entry happens to appear later in the recovered journal.
    if journal
        .entries
        .iter()
        .any(|entry| entry.status == JournalStatus::NeedsReview)
    {
        return Ok(apply_report(&journal, 0));
    }

    let mut device_toggles = 0;
    for index in 0..journal.entries.len() {
        let plan_entry = plan
            .entries
            .iter()
            .find(|entry| entry.instruction_id == journal.entries[index].instruction_id)
            .ok_or_else(|| {
                HsrError::new(
                    "HSR_MANAGER_PLAN_ENTRY_MISSING",
                    JOURNAL_INVALID,
                    "journal instruction is absent from plan",
                )
            })?;

        let expected_before = plan_entry.observed_before.ok_or_else(|| {
            HsrError::new(
                "HSR_MANAGER_PLAN_EVIDENCE_INVALID",
                CONFIRMATION_REQUIRED,
                "an actionable plan entry lacks its complete observed before-state",
            )
        })?;
        if expected_before.lock.is_none() || expected_before.discard.is_none() {
            return Err(HsrError::new(
                "HSR_MANAGER_PLAN_EVIDENCE_INVALID",
                CONFIRMATION_REQUIRED,
                "an actionable plan entry does not contain both managed state flags",
            ));
        }

        match journal.entries[index].status {
            JournalStatus::Verified => continue,
            JournalStatus::NeedsReview => break,
            JournalStatus::MutationStarted => {
                let recovered = read_single_safe_target(device, &plan_entry.matcher);
                let entry = &mut journal.entries[index];
                match recovered {
                    Ok(observation)
                        if entry
                            .change
                            .scope
                            .has_required_lock_evidence(Some(&observation.state))
                            && observed_field(&observation.state, entry.change.field)
                                == Some(entry.change.desired)
                            && collateral_state_matches(
                                &observation.state,
                                &expected_before,
                                entry.change.field,
                            ) =>
                    {
                        entry.status = JournalStatus::Verified;
                        entry.outcome_code = Some("recovered_desired_state".to_owned());
                    },
                    _ => {
                        // The previous click may or may not have happened. Never
                        // infer safety from seeing the old state and never retry.
                        entry.status = JournalStatus::NeedsReview;
                        entry.outcome_code = Some("interrupted_mutation_ambiguous".to_owned());
                    },
                }
                journal_store.save(&journal).map_err(journal_io_error)?;
                if journal.entries[index].status == JournalStatus::NeedsReview {
                    break;
                }
            },
            JournalStatus::Pending => {
                let fresh = match read_single_safe_target(device, &plan_entry.matcher) {
                    Ok(fresh) => fresh,
                    Err(code) => {
                        let entry = &mut journal.entries[index];
                        entry.status = JournalStatus::NeedsReview;
                        entry.outcome_code = Some(code);
                        journal_store.save(&journal).map_err(journal_io_error)?;
                        break;
                    },
                };
                let change = &journal.entries[index].change;
                let current = observed_field(&fresh.state, change.field);
                if !change.scope.has_required_lock_evidence(Some(&fresh.state)) {
                    let entry = &mut journal.entries[index];
                    entry.status = JournalStatus::NeedsReview;
                    entry.outcome_code = Some("mark_discard_locked_or_lock_unknown".to_owned());
                    journal_store.save(&journal).map_err(journal_io_error)?;
                    break;
                }
                if !collateral_state_matches(&fresh.state, &expected_before, change.field)
                    || expected_before.lock.is_none()
                    || expected_before.discard.is_none()
                {
                    let entry = &mut journal.entries[index];
                    entry.status = JournalStatus::NeedsReview;
                    entry.outcome_code = Some("before_state_changed_or_unknown".to_owned());
                    journal_store.save(&journal).map_err(journal_io_error)?;
                    break;
                }
                if current == Some(change.desired) {
                    let entry = &mut journal.entries[index];
                    entry.status = JournalStatus::Verified;
                    entry.outcome_code = Some("already_desired_on_reread".to_owned());
                    journal_store.save(&journal).map_err(journal_io_error)?;
                    continue;
                }
                if current != Some(change.before) {
                    let entry = &mut journal.entries[index];
                    entry.status = JournalStatus::NeedsReview;
                    entry.outcome_code = Some("before_state_changed_or_unknown".to_owned());
                    journal_store.save(&journal).map_err(journal_io_error)?;
                    break;
                }

                {
                    let entry = &mut journal.entries[index];
                    entry.status = JournalStatus::MutationStarted;
                    entry.toggle_attempts = 1;
                    entry.outcome_code = None;
                }
                // No UI input can occur until the durable pre-click record is
                // independently reloadable as the exact intended snapshot.
                save_and_reload_exact(journal_store, &journal)?;

                device_toggles += 1;
                let toggle_result = device.toggle_once(&fresh, journal.entries[index].change.scope);
                let post = read_single_safe_target(device, &plan_entry.matcher);
                let entry = &mut journal.entries[index];
                if matches!(post, Ok(ref observation)
                if entry.change.scope.has_required_lock_evidence(Some(&observation.state))
                    && observed_field(&observation.state, entry.change.field) == Some(entry.change.desired)
                    && collateral_state_matches(
                        &observation.state,
                        &expected_before,
                        entry.change.field,
                    ))
                {
                    entry.status = JournalStatus::Verified;
                    entry.outcome_code = Some(if toggle_result.is_ok() {
                        "postverified".to_owned()
                    } else {
                        "postverified_after_device_error".to_owned()
                    });
                } else {
                    entry.status = JournalStatus::NeedsReview;
                    entry.outcome_code = Some(if toggle_result.is_ok() {
                        "postverification_failed".to_owned()
                    } else {
                        "device_error_state_ambiguous".to_owned()
                    });
                }
                journal_store.save(&journal).map_err(journal_io_error)?;
                if journal.entries[index].status == JournalStatus::NeedsReview {
                    break;
                }
            },
        }
    }

    Ok(apply_report(&journal, device_toggles))
}

fn apply_report(journal: &ManagerJournal, device_toggles: usize) -> ApplyReport {
    ApplyReport {
        original_request_id: journal.plan.request_id.clone(),
        submitted_request_id: journal.plan.request_id.clone(),
        total_actions: journal.entries.len(),
        verified_actions: journal
            .entries
            .iter()
            .filter(|entry| entry.status == JournalStatus::Verified)
            .count(),
        needs_review_actions: journal
            .entries
            .iter()
            .filter(|entry| entry.status == JournalStatus::NeedsReview)
            .count(),
        device_toggles,
    }
}

fn save_and_reload_exact<S: ManagerJournalStore>(
    journal_store: &mut S,
    journal: &ManagerJournal,
) -> HsrResult<()> {
    journal_store.save(journal).map_err(journal_io_error)?;
    let reloaded = journal_store.load().map_err(journal_io_error)?;
    if reloaded.as_ref() != Some(journal) {
        return Err(HsrError::new(
            "HSR_MANAGER_JOURNAL_DURABILITY_INVALID",
            JOURNAL_INVALID,
            "durable manager journal did not reload as the exact pre-input snapshot",
        ));
    }
    Ok(())
}

fn validate_persisted_plan_for_envelope(
    journal: &ManagerJournal,
    envelope: &ManagerInstructionsEnvelope,
) -> HsrResult<()> {
    validate_plan_digest(&journal.plan)?;
    journal.validate_for_plan(&journal.plan)?;
    if journal.plan.idempotency_key != envelope.idempotency_key
        || journal.plan.reference != envelope.reference
        || journal.plan.entries.len() != envelope.instructions.len()
    {
        return Err(HsrError::new(
            "HSR_MANAGER_JOURNAL_ENVELOPE_MISMATCH",
            JOURNAL_INVALID,
            "persisted plan does not belong to this exact semantic instruction set and reference",
        ));
    }

    for entry in &journal.plan.entries {
        let instruction = envelope
            .instructions
            .iter()
            .find(|instruction| instruction.id == entry.instruction_id)
            .ok_or_else(|| {
                HsrError::new(
                    "HSR_MANAGER_JOURNAL_ENVELOPE_MISMATCH",
                    JOURNAL_INVALID,
                    "persisted plan contains an instruction absent from this envelope",
                )
            })?;
        if entry.matcher != instruction.matcher
            || entry.requested_change != requested_change(instruction)
        {
            return Err(HsrError::new(
                "HSR_MANAGER_JOURNAL_ENVELOPE_MISMATCH",
                JOURNAL_INVALID,
                "persisted plan matcher or requested transition differs from this envelope",
            ));
        }

        if let Some(state) = entry.observed_before {
            let observation = ManagedGearObservation {
                matcher: entry.matcher.clone(),
                state,
                equipped: entry.observed_equipped,
            };
            observation.validate()?;
            let expected = plan_instruction(instruction, &[observation]);
            if expected != *entry {
                return Err(HsrError::new(
                    "HSR_MANAGER_JOURNAL_PLAN_INVALID",
                    JOURNAL_INVALID,
                    "persisted plan entry is inconsistent with its recorded visible evidence",
                ));
            }
        } else if !matches!(
            entry.classification,
            PlanClassification::PreviewOnlyNotFound | PlanClassification::PreviewOnlyAmbiguous
        ) || entry.observed_equipped.is_some()
            || !entry.changes.is_empty()
        {
            return Err(HsrError::new(
                "HSR_MANAGER_JOURNAL_PLAN_INVALID",
                JOURNAL_INVALID,
                "persisted plan entry lacks evidence for its classification",
            ));
        }
    }
    Ok(())
}

fn validate_plan_digest(plan: &ManagerPlan) -> HsrResult<()> {
    let recomputed = compute_plan_digest(
        &plan.request_id,
        &plan.idempotency_key,
        &plan.reference,
        &plan.inventory_fingerprint,
        &plan.entries,
    )?;
    if recomputed != plan.digest {
        return Err(HsrError::new(
            "HSR_MANAGER_PLAN_DIGEST_INVALID",
            CONFIRMATION_REQUIRED,
            "plan content no longer matches its preview digest",
        ));
    }
    Ok(())
}

fn validate_plan_action_safety(plan: &ManagerPlan) -> HsrResult<()> {
    if plan.entries.iter().any(|entry| {
        entry.changes.iter().any(|change| {
            !change
                .scope
                .has_required_lock_evidence(entry.observed_before.as_ref())
        })
    }) {
        return Err(HsrError::new(
            "HSR_MANAGER_PLAN_EVIDENCE_INVALID",
            CONFIRMATION_REQUIRED,
            "mark-discard requires explicit unlocked evidence in the confirmed plan",
        ));
    }
    Ok(())
}

fn require_exclusive_apply_lease<S: ManagerJournalStore>(store: &S) -> HsrResult<()> {
    if !store.holds_exclusive_apply_lease() {
        return Err(HsrError::new(
            "HSR_MANAGER_EXCLUSIVE_LEASE_REQUIRED",
            JOURNAL_BUSY,
            "manager apply requires a journal store holding an exclusive cross-process lease",
        ));
    }
    Ok(())
}

fn authorize(plan: &ManagerPlan, authorization: &ApplyAuthorization) -> HsrResult<()> {
    if authorization.confirmed_plan_digest != plan.digest {
        return Err(HsrError::new(
            "HSR_MANAGER_CONFIRMATION_MISMATCH",
            CONFIRMATION_REQUIRED,
            "confirmed digest does not exactly equal the current preview digest",
        ));
    }
    let missing = plan
        .required_scopes()
        .difference(&authorization.scopes)
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(HsrError::new(
            "HSR_MANAGER_SCOPE_MISSING",
            CONFIRMATION_REQUIRED,
            format!("missing explicit authorization scope(s): {missing:?}"),
        ));
    }
    Ok(())
}

fn read_single_safe_target<D: ManagerMutationDevice>(
    device: &mut D,
    matcher: &VisibleGearMatcher,
) -> Result<ManagedGearObservation, String> {
    let observations = device
        .reread(matcher)
        .map_err(|_| "device_reread_failed".to_owned())?
        .into_iter()
        .filter(|observation| matcher_matches(matcher, &observation.matcher))
        .collect::<Vec<_>>();
    if observations.len() != 1 {
        return Err(if observations.is_empty() {
            "fresh_match_not_found".to_owned()
        } else {
            "fresh_match_ambiguous".to_owned()
        });
    }
    let observation = observations.into_iter().next().expect("length checked");
    observation
        .validate()
        .map_err(|_| "fresh_observation_invalid".to_owned())?;
    if observation.equipped != Some(false) {
        return Err(if observation.equipped == Some(true) {
            "fresh_item_equipped".to_owned()
        } else {
            "fresh_equipped_state_unknown".to_owned()
        });
    }
    Ok(observation)
}

fn observed_field(state: &ManagedState, field: ManagedField) -> Option<bool> {
    match field {
        ManagedField::Lock => state.lock,
        ManagedField::Discard => state.discard,
    }
}

fn collateral_state_matches(
    current: &ManagedState,
    expected_before: &ManagedState,
    changed_field: ManagedField,
) -> bool {
    match changed_field {
        ManagedField::Lock => {
            current.lock.is_some()
                && expected_before.lock.is_some()
                && current.discard.is_some()
                && current.discard == expected_before.discard
        },
        ManagedField::Discard => {
            current.discard.is_some()
                && expected_before.discard.is_some()
                && current.lock.is_some()
                && current.lock == expected_before.lock
        },
    }
}

fn matcher_matches(expected: &VisibleGearMatcher, actual: &VisibleGearMatcher) -> bool {
    expected.key == actual.key
        && expected.game_id == actual.game_id
        && expected.set_key == actual.set_key
        && expected.slot == actual.slot
        && expected.rarity == actual.rarity
        && expected.level == actual.level
        && stat_matches(&expected.main_stat, &actual.main_stat)
        && expected.substats.len() == actual.substats.len()
        && expected
            .substats
            .iter()
            .zip(actual.substats.iter())
            .all(|(left, right)| stat_matches(left, right))
        && expected
            .location_key
            .as_ref()
            .map(|location| actual.location_key.as_ref() == Some(location))
            .unwrap_or(true)
}

fn stat_matches(expected: &VisibleStat, actual: &VisibleStat) -> bool {
    expected.key == actual.key && (expected.value - actual.value).abs() <= 1e-6
}

fn validate_matcher(matcher: &VisibleGearMatcher) -> HsrResult<()> {
    validate_identifier("matcher.key", &matcher.key)?;
    if matcher.game_id == 0 {
        return invalid("matcher.gameId must be a public non-zero gear-template ID");
    }
    validate_identifier("matcher.setKey", &matcher.set_key)?;
    if !(1..=5).contains(&matcher.rarity) {
        return invalid("matcher.rarity must be between 1 and 5");
    }
    if matcher.level > 15 {
        return invalid("matcher.level must be between 0 and 15");
    }
    validate_stat("matcher.mainStat", &matcher.main_stat)?;
    if matcher.substats.len() > 4 {
        return invalid("matcher.substats cannot contain more than four visible stats");
    }
    let mut previous: Option<&VisibleStat> = None;
    for stat in &matcher.substats {
        validate_stat("matcher.substats", stat)?;
        if stat.key == matcher.main_stat.key {
            return invalid("matcher.substats cannot repeat the main-stat key");
        }
        if let Some(previous) = previous {
            if previous.key >= stat.key {
                return invalid("matcher.substats must have unique keys sorted in ascending order");
            }
        }
        previous = Some(stat);
    }
    if let Some(location) = matcher.location_key.as_ref() {
        validate_identifier("matcher.locationKey", location)?;
    }
    Ok(())
}

fn validate_stat(field: &str, stat: &VisibleStat) -> HsrResult<()> {
    validate_identifier(&format!("{field}.key"), &stat.key)?;
    if !stat.value.is_finite() {
        return invalid(&format!("{field}.value must be a finite display value"));
    }
    Ok(())
}

fn validate_request_id(value: &str) -> HsrResult<()> {
    validate_stable_id("requestId", value)
}

fn validate_stable_id(field: &str, value: &str) -> HsrResult<()> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 128
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(byte))
    {
        return invalid(&format!(
            "{field} must match [A-Za-z0-9][A-Za-z0-9._:-]* and contain at most 128 ASCII characters"
        ));
    }
    Ok(())
}

fn validate_identifier(field: &str, value: &str) -> HsrResult<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 160 {
        return invalid(&format!("{field} must contain 1 to 160 characters"));
    }
    Ok(())
}

fn reject_sensitive_fields(value: &Value) -> HsrResult<()> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let normalized = key
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>();
                if matches!(
                    normalized.as_str(),
                    "uid"
                        | "accountid"
                        | "accountuid"
                        | "sessionid"
                        | "sessiontoken"
                        | "token"
                        | "cookie"
                        | "authorization"
                        | "serveritemid"
                        | "itemuid"
                        | "inventoryid"
                        | "rawpacket"
                        | "packetpayload"
                        | "payloadbytes"
                ) {
                    return Err(HsrError::new(
                        "HSR_MANAGER_SENSITIVE_FIELD",
                        INVALID_INSTRUCTIONS,
                        "a prohibited account, session, or server-item field name was detected",
                    ));
                }
                reject_sensitive_fields(value)?;
            }
        },
        Value::Array(values) => {
            for value in values {
                reject_sensitive_fields(value)?;
            }
        },
        _ => {},
    }
    Ok(())
}

fn invalid<T>(detail: &str) -> HsrResult<T> {
    Err(HsrError::new(
        "HSR_MANAGER_INSTRUCTIONS_INVALID",
        INVALID_INSTRUCTIONS,
        detail,
    ))
}

fn manager_reference_error(detail: String) -> HsrError {
    HsrError::new(
        "HSR_MANAGER_REFERENCE_MISMATCH",
        INVALID_INSTRUCTIONS,
        detail,
    )
}

fn manager_reference_mismatch<T>(detail: impl Into<String>) -> HsrResult<T> {
    Err(manager_reference_error(detail.into()))
}

fn preview_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "{\"error\":\"invalid preview data\"}".to_owned())
}

fn serialize_javascript_number<S>(value: &f64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if !value.is_finite() {
        return Err(serde::ser::Error::custom(
            "manager stat values must be finite",
        ));
    }
    // JSON.stringify writes finite integral IEEE-754 values such as 5.0 as
    // `5` (and -0.0 as `0`). Matching that behavior is necessary because the
    // GGStarRail idempotency key is computed before the Rust type boundary.
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    if value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}

fn journal_io_error(detail: String) -> HsrError {
    HsrError::new("HSR_MANAGER_JOURNAL_IO", JOURNAL_INVALID, detail)
}

fn compute_instruction_idempotency_key(
    reference_revision: &str,
    instructions: &[ManagerInstruction],
) -> HsrResult<String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct IdempotencyMaterial<'a> {
        reference_revision: &'a str,
        instructions: Vec<&'a ManagerInstruction>,
    }

    // GGStarRail assigns stable instruction IDs after sorting semantic plans.
    // Sorting the received full instruction JSON makes the scanner tolerant of
    // transport/order changes while preserving the settled single-record
    // golden byte-for-byte.
    let mut canonical = instructions
        .iter()
        .map(|instruction| serde_json::to_vec(instruction).map(|bytes| (bytes, instruction)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            HsrError::new(
                "HSR_MANAGER_IDEMPOTENCY_DIGEST_FAILED",
                INVALID_INSTRUCTIONS,
                "could not canonicalize semantic manager instructions",
            )
        })?;
    canonical.sort_by(|left, right| left.0.cmp(&right.0));

    let bytes = serde_json::to_vec(&IdempotencyMaterial {
        reference_revision,
        instructions: canonical
            .into_iter()
            .map(|(_, instruction)| instruction)
            .collect(),
    })
    .map_err(|_| {
        HsrError::new(
            "HSR_MANAGER_IDEMPOTENCY_DIGEST_FAILED",
            INVALID_INSTRUCTIONS,
            "could not serialize semantic manager instruction material",
        )
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn compute_plan_digest(
    request_id: &str,
    idempotency_key: &str,
    reference: &ManagerReference,
    inventory_fingerprint: &str,
    entries: &[ManagerPlanEntry],
) -> HsrResult<String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct DigestMaterial<'a> {
        schema: &'a str,
        schema_version: u32,
        request_id: &'a str,
        idempotency_key: &'a str,
        reference: &'a ManagerReference,
        inventory_fingerprint: &'a str,
        entries: &'a [ManagerPlanEntry],
    }

    let material = DigestMaterial {
        schema: MANAGER_INSTRUCTIONS_SCHEMA,
        schema_version: MANAGER_INSTRUCTIONS_SCHEMA_VERSION,
        request_id,
        idempotency_key,
        reference,
        inventory_fingerprint,
        entries,
    };
    let bytes = serde_json::to_vec(&material).map_err(|_| {
        HsrError::new(
            "HSR_MANAGER_DIGEST_FAILED",
            INVALID_INSTRUCTIONS,
            "could not serialize plan digest material",
        )
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn compute_inventory_fingerprint(inventory: &[ManagedGearObservation]) -> HsrResult<String> {
    let mut observations = inventory
        .iter()
        .map(|observation| {
            serde_json::to_vec(observation).map_err(|_| {
                HsrError::new(
                    "HSR_MANAGER_INVENTORY_DIGEST_FAILED",
                    INVALID_INSTRUCTIONS,
                    "could not serialize a fresh visible inventory observation",
                )
            })
        })
        .collect::<HsrResult<Vec<_>>>()?;
    observations.sort();

    let mut digest = Sha256::new();
    digest.update(b"goodscanner.hsr.manager.inventory.v1\0");
    for observation in observations {
        digest.update((observation.len() as u64).to_be_bytes());
        digest.update(observation);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn is_sha256_key(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
