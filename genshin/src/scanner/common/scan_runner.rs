//! Shared scan orchestration used by the GUI/CLI scanner and the HTTP `/scan` API.
//!
//! The individual scanners own OCR and navigation inside each category. This
//! module owns the cross-category contract: phase order, per-phase progress
//! keys, cancellation behavior, and the weapon-to-artifact skip-open handoff.

use std::sync::Arc;

use anyhow::{Error, Result};
use yas::{log_info, log_warn};

use crate::cli::{GoodScannerApplication, GoodUserConfig, ScanCoreConfig};
use crate::scanner::artifact::GoodArtifactScanner;
use crate::scanner::character::GoodCharacterScanner;
use crate::scanner::common::game_controller::GenshinGameController;
use crate::scanner::common::mappings::MappingManager;
use crate::scanner::common::models::{GoodArtifact, GoodCharacter, GoodWeapon};
use crate::scanner::common::ocr_pool::SharedOcrPools;
use crate::scanner::common::progress::ProgressFn;
use crate::scanner::weapon::GoodWeaponScanner;

/// Result of a single scan phase.
///
/// A phase is `Complete` only if the caller is allowed to publish/export its
/// data. User cancellation, failure, and a skipped start are represented
/// separately so HTTP results can explain a real error without presenting a
/// deliberate stop as a technical failure.
pub enum ScanPhaseResult<T> {
    /// Phase was not requested by the caller.
    NotAttempted,
    /// Phase was requested but was stopped before it finished and should not
    /// be published. This does not carry a technical error.
    Incomplete,
    /// Phase failed. The anyhow error is retained so callers can render the
    /// complete source chain with `{:#}` instead of losing it in a log entry.
    Failed(Error),
    /// Phase finished with publishable data.
    Complete(Vec<T>),
}

impl<T> ScanPhaseResult<T> {
    pub fn into_complete(self) -> Option<Vec<T>> {
        match self {
            Self::Complete(data) => Some(data),
            Self::NotAttempted | Self::Incomplete | Self::Failed(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_due_to_cancel_marks_requested_phases_incomplete() {
        assert!(matches!(
            skipped_due_to_cancel::<GoodCharacter>(true),
            ScanPhaseResult::Incomplete
        ));
        assert!(matches!(
            skipped_due_to_cancel::<GoodCharacter>(false),
            ScanPhaseResult::NotAttempted
        ));
    }

    #[test]
    fn continue_on_error_retains_the_complete_failure_chain() {
        let source = anyhow::anyhow!("inner scan marker / STATUS_MARKER")
            .context("weapon scan phase failed");
        let token = yas::cancel::CancelToken::new();

        let phase = phase_result::<()>(
            Err(source),
            &token,
            ScanRunOptions {
                save_on_cancel: false,
                accept_cancelled_success: false,
                failure_policy: ScanFailurePolicy::ContinueOnError,
            },
            "weapon",
        )
        .unwrap();

        let ScanPhaseResult::Failed(error) = phase else {
            panic!("a real scanner error must remain a failed phase");
        };
        assert_eq!(
            format!("{error:#}"),
            "weapon scan phase failed: inner scan marker / STATUS_MARKER"
        );
    }

    #[test]
    fn user_cancelled_error_is_incomplete_without_a_failure_diagnostic() {
        let token = yas::cancel::CancelToken::new();
        token.cancel(yas::cancel::StopReason::UserAbort);

        let phase = phase_result::<()>(
            Err(anyhow::anyhow!("scanner noticed cancellation")),
            &token,
            ScanRunOptions {
                save_on_cancel: false,
                accept_cancelled_success: false,
                failure_policy: ScanFailurePolicy::ContinueOnError,
            },
            "artifact",
        )
        .unwrap();

        assert!(matches!(phase, ScanPhaseResult::Incomplete));
    }
}

/// Result of a scan execution. Each category reports
/// Complete/Incomplete/Failed/NotAttempted.
pub struct ScanRunResult {
    pub characters: ScanPhaseResult<GoodCharacter>,
    pub weapons: ScanPhaseResult<GoodWeapon>,
    pub artifacts: ScanPhaseResult<GoodArtifact>,
}

/// Whether one failed phase should abort the whole scan or only mark that phase incomplete.
#[derive(Clone, Copy)]
pub enum ScanFailurePolicy {
    StopOnError,
    ContinueOnError,
}

/// Runtime behavior that differs between local exports and HTTP scan jobs.
#[derive(Clone, Copy)]
pub struct ScanRunOptions {
    /// If true, a cancellation error in the active phase becomes an empty
    /// complete phase so local GUI scans can still export already-collected data.
    pub save_on_cancel: bool,
    /// If true, a phase that returns data after cancellation is still publishable.
    /// GUI/CLI exports use this to preserve partial files; HTTP `/scan` keeps
    /// cancelled jobs incomplete so clients do not consume stale cache entries.
    pub accept_cancelled_success: bool,
    pub failure_policy: ScanFailurePolicy,
}

fn scan_cancelled(cancel_token: &yas::cancel::CancelToken, ctrl: &GenshinGameController) -> bool {
    cancel_token.is_cancelled() || ctrl.is_cancelled()
}

fn skipped_due_to_cancel<T>(requested: bool) -> ScanPhaseResult<T> {
    if requested {
        ScanPhaseResult::Incomplete
    } else {
        ScanPhaseResult::NotAttempted
    }
}

/// Execute the requested scan phases with shared scanner setup and phase semantics.
pub fn run_scan_phases(
    ctrl: &mut GenshinGameController,
    mappings: Arc<MappingManager>,
    pools: Arc<SharedOcrPools>,
    user_config: &GoodUserConfig,
    config: &ScanCoreConfig,
    progress_fn: Option<&ProgressFn<'_>>,
    status_fn: Option<&dyn Fn(&str)>,
    cancel_token: yas::cancel::CancelToken,
    options: ScanRunOptions,
) -> Result<ScanRunResult> {
    let scanner_config = config.to_scanner_config();

    ctrl.focus_game_window();
    ctrl.set_cancel_token(cancel_token.clone());

    let report = |msg: &str| {
        if let Some(f) = status_fn {
            f(msg);
        }
    };

    let chars_progress = |c: usize, t: usize, id: &str, _phase: &str| {
        if let Some(outer) = progress_fn {
            outer(c, t, id, "characters");
        }
    };
    let weapons_progress = |c: usize, t: usize, id: &str, _phase: &str| {
        if let Some(outer) = progress_fn {
            outer(c, t, id, "weapons");
        }
    };
    let artifacts_progress = |c: usize, t: usize, id: &str, _phase: &str| {
        if let Some(outer) = progress_fn {
            outer(c, t, id, "artifacts");
        }
    };

    let mut characters = ScanPhaseResult::NotAttempted;
    let mut weapons = ScanPhaseResult::NotAttempted;
    let mut artifacts = ScanPhaseResult::NotAttempted;

    if config.scan_characters {
        characters = if scan_cancelled(&cancel_token, ctrl) {
            ScanPhaseResult::Incomplete
        } else {
            report("扫描角色 / Scanning characters...");
            log_info!("扫描角色...", "Scanning characters...");
            let cfg = GoodScannerApplication::make_char_config(&scanner_config, user_config);
            let scan_result = match GoodCharacterScanner::new(cfg, mappings.clone()) {
                Ok(scanner) => scanner.scan(ctrl, 0, &pools, Some(&chars_progress)),
                Err(e) => Err(e),
            };
            let phase = phase_result(scan_result, &cancel_token, options, "character")?;
            if matches!(phase, ScanPhaseResult::Complete(_)) && !scan_cancelled(&cancel_token, ctrl)
            {
                ctrl.return_to_main_ui(4);
            }
            phase
        };
    }

    if scan_cancelled(&cancel_token, ctrl) {
        return Ok(ScanRunResult {
            characters,
            weapons: skipped_due_to_cancel(config.scan_weapons),
            artifacts: skipped_due_to_cancel(config.scan_artifacts),
        });
    }

    if config.scan_weapons {
        report("扫描武器 / Scanning weapons...");
        log_info!("扫描武器...", "Scanning weapons...");
        let cfg = GoodScannerApplication::make_weapon_config(&scanner_config, user_config);
        let scan_result = match GoodWeaponScanner::new(cfg, mappings.clone()) {
            Ok(scanner) => scanner.scan(ctrl, false, 0, &pools, Some(&weapons_progress)),
            Err(e) => Err(e),
        };
        weapons = phase_result(scan_result, &cancel_token, options, "weapon")?;
    }

    if scan_cancelled(&cancel_token, ctrl) {
        return Ok(ScanRunResult {
            characters,
            weapons,
            artifacts: skipped_due_to_cancel(config.scan_artifacts),
        });
    }

    if config.scan_artifacts {
        report("扫描圣遗物 / Scanning artifacts...");
        log_info!("扫描圣遗物...", "Scanning artifacts...");
        let cfg = GoodScannerApplication::make_artifact_config(&scanner_config, user_config);
        let skip_open = matches!(weapons, ScanPhaseResult::Complete(_));
        let scan_result = match GoodArtifactScanner::new(cfg, mappings.clone()) {
            Ok(scanner) => scanner.scan(ctrl, skip_open, 0, &pools, Some(&artifacts_progress)),
            Err(e) => Err(e),
        };
        artifacts = phase_result(scan_result, &cancel_token, options, "artifact")?;
    }

    Ok(ScanRunResult {
        characters,
        weapons,
        artifacts,
    })
}

fn phase_result<T>(
    result: Result<Vec<T>>,
    cancel_token: &yas::cancel::CancelToken,
    options: ScanRunOptions,
    phase_name: &str,
) -> Result<ScanPhaseResult<T>> {
    match result {
        Ok(data) if options.accept_cancelled_success || !cancel_token.is_cancelled() => {
            Ok(ScanPhaseResult::Complete(data))
        },
        Ok(_) => Ok(ScanPhaseResult::Incomplete),
        Err(e)
            if matches!(
                cancel_token.reason(),
                Some(yas::cancel::StopReason::UserAbort)
            ) =>
        {
            log_info!("阶段被用户中断: {:#}", "Phase aborted by user: {:#}", e);
            if options.save_on_cancel {
                Ok(ScanPhaseResult::Complete(Vec::new()))
            } else {
                Ok(ScanPhaseResult::Incomplete)
            }
        },
        Err(e) => {
            log_warn!(
                "[scan] {}阶段失败: {:#}",
                "[scan] {} phase failed: {:#}",
                phase_name,
                e
            );
            match options.failure_policy {
                ScanFailurePolicy::StopOnError => Err(e),
                ScanFailurePolicy::ContinueOnError => Ok(ScanPhaseResult::Failed(e)),
            }
        },
    }
}
