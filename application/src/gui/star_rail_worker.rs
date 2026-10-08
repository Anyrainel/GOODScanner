use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use hsr_scanner::{
    data_cache::load_data_cache,
    export_observations,
    manager::{
        apply_manager_envelope, build_manager_plan, load_manager_recovery_plan,
        validate_manager_envelope_reference, AppendOnlyJsonJournalStore, ApplyAuthorization,
        HsrControllerLease, ManagerInstructionsEnvelope, ManagerJournalStore, ManagerPlan,
        MutationScope,
    },
    pipeline::write_json_create_new,
    reference::ReferenceCache,
    scanner::{HsrScanner, ScanConfig, ScanTargets},
    CaptureExportDetails, HsrError, Language, ValidatedObservationSnapshot,
};

use crate::config::StarRailSettings;

use super::{
    star_rail_state::ManagerPreview,
    state::{LogSource, TaskKind, TaskStatus, UiError, UiText},
    worker::{self, TaskHandle},
};

const MAX_MANAGER_INSTRUCTIONS_BYTES: u64 = 16 * 1024 * 1024;

fn manager_scan_observer(
    progress: Arc<Mutex<super::task_progress::TaskProgress>>,
) -> hsr_scanner::scan_progress::ScanObserver {
    Box::new(move |category, event| {
        if category != hsr_scanner::scan_progress::ScanCategory::Gear {
            return;
        }
        if let hsr_scanner::scan_progress::ScanEvent::Progress {
            recognized, total, ..
        } = event
        {
            if let Some(step) = progress
                .lock()
                .unwrap()
                .steps
                .iter_mut()
                .find(|s| s.key == "gear")
            {
                step.completed = recognized;
                step.total = total;
                step.state = super::task_progress::StepState::Running;
            }
        }
    })
}

pub fn settings_identity(settings: &StarRailSettings) -> String {
    serde_json::to_string(settings).unwrap_or_default()
}

fn hsr_language() -> Language {
    if yas::lang::is_en() {
        Language::En
    } else {
        Language::ZhCn
    }
}

fn user_aborted(cancel: &yas::cancel::CancelToken) -> bool {
    cancel.reason() == Some(yas::cancel::StopReason::UserAbort)
}

fn stopped(task: TaskKind) -> UiText {
    match task {
        TaskKind::Scanner => UiText::new("扫描已安全停止。", "The scan stopped safely."),
        TaskKind::Manager => UiText::new(
            "管理器预览已安全停止；未执行任何变更。",
            "The manager preview stopped safely; no changes were made.",
        ),
        TaskKind::Capture => UiText::new("抓包已安全停止。", "Capture stopped safely."),
    }
}

pub(super) fn hsr_ui_error(hint: UiText, error: HsrError) -> UiError {
    let language = hsr_language();
    let actionable = error.hint();
    UiError::from_message(
        UiText::new(
            actionable.select(hsr_scanner::localization::Language::ZhCn),
            actionable.select(hsr_scanner::localization::Language::En),
        ),
        format!(
            "{}\n{}",
            hint.text(match language {
                hsr_scanner::localization::Language::ZhCn => super::state::Lang::Zh,
                hsr_scanner::localization::Language::En => super::state::Lang::En,
            }),
            error.localized_message(language)
        ),
    )
}

fn ensure_ocr_runtime() -> Result<(), UiError> {
    ensure_ocr_runtime_with_status(None)
}

fn ensure_ocr_runtime_with_status(status: Option<&Arc<Mutex<TaskStatus>>>) -> Result<(), UiError> {
    #[cfg(target_os = "windows")]
    {
        genshin_scanner::cli::check_vcpp_runtime().map_err(|error| {
            UiError::from_anyhow(
                UiText::new(
                    "星穹铁道扫描器无法启动，因为所需的 Windows 运行组件不可用。请按完整错误中的说明修复后重试。",
                    "The Star Rail scanner cannot start because a required Windows runtime component is unavailable. Follow the full error details, then retry.",
                ),
                &error,
            )
        })?;
        if !genshin_scanner::cli::check_onnxruntime() {
            if let Some(status) = status {
                *status.lock().unwrap() =
                    TaskStatus::Running(UiText::new("正在下载 OCR 引擎", "Downloading OCR engine"));
            }
            genshin_scanner::cli::download_onnxruntime().map_err(|error| {
                UiError::from_anyhow(
                    UiText::new(
                        "星穹铁道扫描器无法下载 OCR 引擎。请检查网络连接或安全软件设置，然后重试。",
                        "The Star Rail scanner could not download its OCR engine. Check the network connection or security software, then retry.",
                    ),
                    &error,
                )
            })?;
        }
    }
    Ok(())
}

fn scanner_config(
    settings: &StarRailSettings,
    targets: ScanTargets,
) -> Result<ScanConfig, UiError> {
    let trailblazer = settings.trailblazer();
    if targets.characters && trailblazer.is_none() {
        return Err(UiError::from_message(
            UiText::new(
                "扫描角色前，请填写开拓者的游戏内昵称并选择性别。",
                "Before scanning Characters, enter the Trailblazer's in-game nickname and choose a gender.",
            ),
            "starRail.trailblazerName or starRail.trailblazerGender is empty",
        ));
    }

    Ok(ScanConfig {
        targets,
        capture_method: settings.capture_method.to_yas(),
        hdr_mode: settings.hdr_mode,
        timings: settings.timings.clone(),
        max_light_cones: settings.max_light_cones,
        max_gear: settings.max_gear,
        max_characters: settings.max_characters,
        trailblazer,
        dump_images: settings.dump_images,
        ..ScanConfig::default()
    })
}

fn ensure_output_dir(settings: &StarRailSettings) -> Result<PathBuf, UiError> {
    let output_dir = settings.export_directory();
    fs::create_dir_all(&output_dir).map_err(|error| {
        UiError::from_error(
            UiText::new(
                "无法创建星穹铁道导出文件夹。请检查路径和文件权限。",
                "The Star Rail export folder could not be created. Check the path and file permissions.",
            ),
            error,
        )
    })?;
    Ok(output_dir)
}

/// All app flows share the hosted HSR reference and its validated local cache.
pub fn load_references() -> Result<ReferenceCache, HsrError> {
    let references = load_data_cache()?;
    references.validate_live_complete_profile()?;
    Ok(references)
}

pub fn next_scan_export_path(output_dir: &Path) -> PathBuf {
    output_dir.join(format!(
        "star_rail_scan_{}.json",
        genshin_scanner::cli::chrono_timestamp()
    ))
}

struct V4ExportFile {
    path: PathBuf,
    counts: (usize, usize, usize, usize),
}

fn write_v4_export(
    observations: &ValidatedObservationSnapshot,
    references: &ReferenceCache,
    output_dir: &Path,
    details: &CaptureExportDetails,
) -> Result<V4ExportFile, UiError> {
    let export = export_observations(observations, references, details, None).map_err(|error| {
        hsr_ui_error(
            UiText::new(
                "扫描结果无法转换为 HSR-Scanner v4 JSON。",
                "The scan result could not be converted into HSR-Scanner v4 JSON.",
            ),
            error,
        )
    })?;
    let path = next_scan_export_path(output_dir);
    write_json_create_new(&path, &export).map_err(|error| {
        hsr_ui_error(
            UiText::new(
                "星穹铁道导出文件无法写入。请检查输出文件夹、磁盘空间和文件权限。",
                "The Star Rail export file could not be written. Check the output folder, disk space, and file permissions.",
            ),
            error,
        )
    })?;
    Ok(V4ExportFile {
        counts: v4_inventory_counts(&export),
        path,
    })
}

fn v4_inventory_counts(export: &serde_json::Value) -> (usize, usize, usize, usize) {
    let characters = export["characters"].as_array().map(Vec::len).unwrap_or(0);
    let light_cones = export["light_cones"].as_array().map(Vec::len).unwrap_or(0);
    let relics = export["relics"].as_array();
    let planar = relics
        .map(|items| {
            items
                .iter()
                .filter(|item| matches!(item["slot"].as_str(), Some("Planar Sphere" | "Link Rope")))
                .count()
        })
        .unwrap_or(0);
    let cavern = relics.map(Vec::len).unwrap_or(0).saturating_sub(planar);
    (characters, light_cones, cavern, planar)
}

fn v4_export_message(counts: (usize, usize, usize, usize), path: &Path) -> UiText {
    UiText::new(
        format!(
            "已导出：{} 个角色、{} 个光锥、{} 件隧洞遗器、{} 件位面饰品。输出：{}",
            counts.0, counts.1, counts.2, counts.3, path.display()
        ),
        format!(
            "Exported {} Characters, {} Light Cones, {} Cavern Relics, and {} Planar Ornaments. Output: {}",
            counts.0, counts.1, counts.2, counts.3, path.display()
        ),
    )
}

/// Preserve a fully reconciled journal under a unique completed name so the
/// configured base path can safely serve the next distinct instruction set.
/// Incomplete and needs-review journals must never call this function.
pub fn archive_completed_manager_journal(path: &Path) -> Result<Option<PathBuf>, UiError> {
    if !path.exists() {
        return Ok(None);
    }
    let timestamp = genshin_scanner::cli::chrono_timestamp();
    for sequence in 0..1_000_u16 {
        let suffix = if sequence == 0 {
            format!(".completed-{timestamp}")
        } else {
            format!(".completed-{timestamp}-{sequence}")
        };
        let mut archive_name = path.as_os_str().to_os_string();
        archive_name.push(suffix);
        let archive_path = PathBuf::from(archive_name);
        if archive_path.exists() {
            continue;
        }
        match fs::rename(path, &archive_path) {
            Ok(()) => return Ok(Some(archive_path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(UiError::from_error(
                    UiText::new(
                        "所有变更均已验证，但恢复日志无法安全归档。在导入其他指令前，请先选择新的恢复日志路径。",
                        "All changes were verified, but the recovery journal could not be archived safely. Choose a new recovery-journal path before loading different instructions.",
                    ),
                    error,
                ));
            },
        }
    }
    Err(UiError::from_message(
        UiText::new(
            "所有变更均已验证，但找不到可用的恢复日志归档名。请选择新的恢复日志路径。",
            "All changes were verified, but no available recovery-journal archive name was found. Choose a new recovery-journal path.",
        ),
        format!("completed journal archive namespace exhausted; path={}", path.display()),
    ))
}

pub fn spawn_scan(
    settings: &StarRailSettings,
    status: Arc<Mutex<TaskStatus>>,
    progress: Arc<Mutex<super::task_progress::TaskProgress>>,
) -> TaskHandle {
    use super::task_progress::{Step, TaskProgress};
    let mut steps = Vec::new();
    for (enabled, key, zh, en) in [
        (settings.scan_characters, "characters", "角色", "Characters"),
        (
            settings.scan_light_cones,
            "light_cones",
            "光锥",
            "Light Cones",
        ),
        (
            settings.scan_relics_and_ornaments,
            "gear",
            "遗器与位面饰品",
            "Relics & Ornaments",
        ),
    ] {
        if enabled {
            steps.push(Step::new(key, zh, en));
        }
    }
    *progress.lock().unwrap() = TaskProgress { steps };
    let settings = settings.clone();
    let status_for_progress = status.clone();
    let status_for_phases = status.clone();
    worker::spawn_cancellable_task(
        TaskKind::Scanner,
        LogSource::Scanner,
        status,
        UiText::new(
            "正在初始化星穹铁道扫描器...",
            "Initializing Star Rail scanner...",
        ),
        UiText::new(
            "正在安全停止星穹铁道扫描...",
            "Stopping the Star Rail scan safely...",
        ),
        move |cancel| {
            let track = progress.clone();
            let cancel_for_observer = cancel.clone();
            let observer = Box::new(move |category, event| {
                use super::task_progress::StepState;
                use hsr_scanner::scan_progress::{ScanCategory, ScanEvent};
                let key = match category {
                    ScanCategory::Characters => "characters",
                    ScanCategory::LightCones => "light_cones",
                    ScanCategory::Gear => "gear",
                };
                if let Some(step) = track
                    .lock()
                    .unwrap()
                    .steps
                    .iter_mut()
                    .find(|s| s.key == key)
                {
                    match event {
                        ScanEvent::Started => {
                            step.state = StepState::Running;
                            if !cancel_for_observer.is_cancelled() {
                                *status_for_progress.lock().unwrap() =
                                    TaskStatus::Running(UiText::new(
                                        format!("正在扫描{}", step.zh),
                                        format!("Scanning {}", step.en),
                                    ));
                            }
                        },
                        ScanEvent::Progress {
                            recognized, total, ..
                        } => {
                            step.completed = recognized;
                            step.total = total;
                        },
                        ScanEvent::Finished {
                            recognized,
                            complete,
                        } => {
                            step.completed = recognized;
                            step.state = if complete {
                                StepState::Complete
                            } else {
                                StepState::Interrupted
                            };
                        },
                    }
                }
            });
            let result = run_scan_observed(
                &settings,
                cancel,
                None,
                Some(observer),
                Some(&status_for_phases),
            );
            progress.lock().unwrap().interrupt_unfinished();
            result
        },
    )
}

/// The GUI and shipped command line use the same runtime, lease, scan, and export path.
pub(crate) fn run_scan(
    settings: &StarRailSettings,
    cancel: yas::cancel::CancelToken,
    sample_limit: Option<usize>,
) -> Result<UiText, UiError> {
    run_scan_observed(settings, cancel, sample_limit, None, None).map(|status| match status {
        TaskStatus::Exported { message, .. } | TaskStatus::Stopped(message) => message,
        _ => unreachable!("scan returns an export or a stopped outcome"),
    })
}

fn run_scan_observed(
    settings: &StarRailSettings,
    cancel: yas::cancel::CancelToken,
    sample_limit: Option<usize>,
    observer: Option<hsr_scanner::scan_progress::ScanObserver>,
    status: Option<&Arc<Mutex<TaskStatus>>>,
) -> Result<TaskStatus, UiError> {
    if user_aborted(&cancel) {
        return Ok(TaskStatus::Stopped(UiText::new(
            "扫描已停止 · 未导出",
            "Scan stopped · no export",
        )));
    }
    let targets = ScanTargets {
        characters: settings.scan_characters,
        light_cones: settings.scan_light_cones,
        gear: settings.scan_relics_and_ornaments,
    };
    let mut config = scanner_config(settings, targets)?;
    config.scan_item_limit = sample_limit;
    let output_dir = ensure_output_dir(settings)?;
    if user_aborted(&cancel) {
        return Ok(TaskStatus::Stopped(UiText::new(
            "扫描已停止 · 未导出",
            "Scan stopped · no export",
        )));
    }
    let phase = |zh, en| {
        if !user_aborted(&cancel) {
            if let Some(status) = status {
                *status.lock().unwrap() = TaskStatus::Running(UiText::new(zh, en));
            }
        }
    };
    phase("正在准备 OCR 引擎", "Preparing OCR engine");
    ensure_ocr_runtime_with_status(status)?;
    phase("正在加载游戏数据", "Loading game data");
    let references = load_references().map_err(|error| {
        hsr_ui_error(
            UiText::new(
                "无法加载星穹铁道游戏数据。请重新下载最新版本的程序后重试。",
                "Star Rail game data could not be loaded. Download the latest app build and retry.",
            ),
            error,
        )
    })?;
    if user_aborted(&cancel) {
        return Ok(TaskStatus::Stopped(UiText::new(
            "扫描已停止 · 未导出",
            "Scan stopped · no export",
        )));
    }
    let _controller_lease = HsrControllerLease::try_acquire().map_err(|error| {
        hsr_ui_error(
            UiText::new(
                "另一个星穹铁道操作正在使用游戏控制器。请等待完成后重试。",
                "Another Star Rail operation is using the game controller. Wait for it to finish, then retry.",
            ),
            error,
        )
    })?;
    phase("正在连接游戏", "Connecting to the game");
    let result = match HsrScanner::live_with_cancel(references.clone(), config, cancel.clone())
        .map(|scanner| match observer {
            Some(observer) => scanner.with_observer(observer),
            None => scanner,
        })
        .and_then(HsrScanner::scan)
    {
        Ok(result) => result,
        Err(error)
            if user_aborted(&cancel)
                && matches!(error.code(), "HSR-SCAN-CANCELLED" | "HSR-DEVICE-CANCELLED") =>
        {
            return Ok(TaskStatus::Stopped(UiText::new(
                "扫描已停止 · 未导出",
                "Scan stopped · no export",
            )))
        },
        Err(error) => {
            return Err(hsr_ui_error(
                UiText::new(
                    "星穹铁道扫描未能完成。",
                    "The Star Rail scan could not finish.",
                ),
                error,
            ));
        },
    };
    if user_aborted(&cancel) {
        return Ok(TaskStatus::Stopped(UiText::new(
            "扫描已停止 · 未导出",
            "Scan stopped · no export",
        )));
    }
    phase("正在导出数据", "Saving export");
    let export = write_v4_export(
        &result.observations,
        &references,
        &output_dir,
        &result.export_details,
    )?;
    let partial = (targets.characters
        && result.coverage.characters != hsr_scanner::model::CoverageLevel::Complete)
        || (targets.light_cones
            && result.coverage.light_cones != hsr_scanner::model::CoverageLevel::Complete)
        || (targets.gear && result.coverage.relics != hsr_scanner::model::CoverageLevel::Complete);
    Ok(TaskStatus::Exported {
        message: v4_export_message(export.counts, &export.path),
        path: export.path.display().to_string(),
        partial,
    })
}

fn load_manager_instructions(path: &Path) -> Result<ManagerInstructionsEnvelope, UiError> {
    let metadata = fs::metadata(path).map_err(|error| {
        UiError::from_error(
            UiText::new(
                "无法读取 GGStarRail 管理指令文件。",
                "The GGStarRail manager-instructions file could not be read.",
            ),
            error,
        )
    })?;
    if !metadata.is_file() || metadata.len() > MAX_MANAGER_INSTRUCTIONS_BYTES {
        return Err(UiError::from_message(
            UiText::new(
                "GGStarRail 管理指令文件无效或过大；不会执行任何游戏操作。",
                "The GGStarRail manager-instructions file is invalid or too large; no game action was performed.",
            ),
            format!(
                "path={}; regularFile={}; bytes={}; maximum={MAX_MANAGER_INSTRUCTIONS_BYTES}",
                path.display(),
                metadata.is_file(),
                metadata.len()
            ),
        ));
    }
    let json = fs::read_to_string(path).map_err(|error| {
        UiError::from_error(
            UiText::new(
                "无法读取 GGStarRail 管理指令文件。",
                "The GGStarRail manager-instructions file could not be read.",
            ),
            error,
        )
    })?;
    ManagerInstructionsEnvelope::parse_json(&json).map_err(|error| {
        hsr_ui_error(
            UiText::new(
                "GGStarRail 管理指令未通过安全校验；不会执行任何游戏操作。",
                "The GGStarRail manager instructions did not pass safety validation; no game action was performed.",
            ),
            error,
        )
    })
}

fn build_manager_preview(
    plan: ManagerPlan,
    settings_identity: String,
    recovered: bool,
) -> Result<ManagerPreview, UiError> {
    let exact_json = serde_json::to_string_pretty(&plan).map_err(|error| {
        UiError::from_error(
            UiText::new(
                "无法显示精确预览；不会执行任何游戏操作。",
                "The exact preview could not be displayed; no game action was performed.",
            ),
            error,
        )
    })?;
    Ok(ManagerPreview {
        plan,
        exact_json,
        recovered,
        settings_identity,
    })
}

/// Load and render the journal's exact original plan without touching the
/// game. Keeping this adapter pure makes restart recovery testable at the UI
/// boundary before a controller or OCR device is created.
pub fn load_recovery_preview<S: ManagerJournalStore>(
    envelope: &ManagerInstructionsEnvelope,
    journal: &mut S,
    settings_identity: String,
) -> Result<Option<ManagerPreview>, UiError> {
    load_manager_recovery_plan(envelope, journal)
        .map_err(|error| {
            hsr_ui_error(
                UiText::new(
                    "恢复日志无法安全载入原始预览；不会执行任何游戏操作。",
                    "The recovery journal could not safely load its original preview; no game action was performed.",
                ),
                error,
            )
        })?
        .map(|plan| build_manager_preview(plan, settings_identity, true))
        .transpose()
}

fn actionable_preview_count(preview: &ManagerPreview) -> usize {
    preview
        .plan
        .entries
        .iter()
        .filter(|entry| !entry.changes.is_empty())
        .count()
}

fn publish_manager_preview(slot: &Mutex<Option<ManagerPreview>>, preview: ManagerPreview) -> usize {
    let actionable = actionable_preview_count(&preview);
    match slot.lock() {
        Ok(mut stored) => *stored = Some(preview),
        Err(poisoned) => {
            slot.clear_poison();
            *poisoned.into_inner() = Some(preview);
        },
    }
    actionable
}

fn manager_scan_config(settings: &StarRailSettings) -> Result<ScanConfig, UiError> {
    let mut config = scanner_config(
        settings,
        ScanTargets {
            characters: false,
            light_cones: false,
            gear: true,
        },
    )?;
    // Sample caps belong to export scans. Manager matching needs the complete
    // inventory and must not inherit a user's diagnostic sample preference.
    config.max_gear = 0;
    Ok(config)
}

pub fn spawn_manager_preview(
    settings: &StarRailSettings,
    status: Arc<Mutex<TaskStatus>>,
    preview: Arc<Mutex<Option<ManagerPreview>>>,
    progress: Arc<Mutex<super::task_progress::TaskProgress>>,
) -> TaskHandle {
    let settings = settings.clone();
    let identity = settings_identity(&settings);
    worker::spawn_cancellable_task(
        TaskKind::Manager,
        LogSource::Manager,
        status,
        UiText::new(
            "正在重新扫描遗器并生成精确预览...",
            "Rescanning Relics and building an exact preview...",
        ),
        UiText::new(
            "正在安全停止管理器预览...",
            "Stopping the manager preview safely...",
        ),
        move |cancel| {
            if user_aborted(&cancel) {
                return Ok(stopped(TaskKind::Manager));
            }
            let instructions_path = Path::new(settings.manager_instructions_path.trim());
            if settings.manager_journal_path.trim().is_empty() {
                return Err(UiError::from_message(
                    UiText::new(
                        "请选择恢复日志路径；在确认没有需恢复的操作前，不会连接游戏。",
                        "Choose a recovery-journal path. The game will not be accessed until recovery state is checked.",
                    ),
                    "starRail.managerJournalPath is empty",
                ));
            }
            let references = load_references().map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "无法加载星穹铁道游戏数据。请重新下载最新版本的程序后重试。",
                        "Star Rail game data could not be loaded. Download the latest app build and retry.",
                    ),
                    error,
                )
            })?;
            if user_aborted(&cancel) {
                return Ok(stopped(TaskKind::Manager));
            }
            let envelope = load_manager_instructions(instructions_path)?;
            if user_aborted(&cancel) {
                return Ok(stopped(TaskKind::Manager));
            }
            validate_manager_envelope_reference(&envelope, &references).map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "管理指令与当前参考数据不一致；不会执行任何游戏操作。",
                        "The manager instructions do not match the current reference data; no game action was performed.",
                    ),
                    error,
                )
            })?;
            let mut recovery_store = AppendOnlyJsonJournalStore::new(PathBuf::from(
                settings.manager_journal_path.trim(),
            ));
            if let Some(recovered) =
                load_recovery_preview(&envelope, &mut recovery_store, identity.clone())?
            {
                if user_aborted(&cancel) {
                    return Ok(stopped(TaskKind::Manager));
                }
                let actionable = publish_manager_preview(&preview, recovered);
                return Ok(UiText::new(
                    format!(
                        "已从恢复日志载入原始精确预览：{} 项需要重新授权。尚未连接游戏。",
                        actionable
                    ),
                    format!(
                        "Loaded the original exact preview from the recovery journal: {actionable} change(s) require renewed authorization. The game was not accessed."
                    ),
                ));
            }

            ensure_ocr_runtime()?;
            if user_aborted(&cancel) {
                return Ok(stopped(TaskKind::Manager));
            }
            let _controller_lease = HsrControllerLease::try_acquire().map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "另一个星穹铁道操作正在使用游戏控制器；不会执行任何游戏操作。",
                        "Another Star Rail operation is using the game controller; no game action was performed.",
                    ),
                    error,
                )
            })?;
            let config = manager_scan_config(&settings)?;
            let mut step =
                super::task_progress::Step::new("gear", "重扫遗器库存", "Rescan Relic inventory");
            step.state = super::task_progress::StepState::Running;
            progress.lock().unwrap().steps = vec![step];
            let mut scanner = match HsrScanner::live_with_cancel(references, config, cancel.clone())
            {
                Ok(scanner) => scanner,
                Err(_) if user_aborted(&cancel) => return Ok(stopped(TaskKind::Manager)),
                Err(error) => {
                    return Err(
                    hsr_ui_error(
                        UiText::new(
                            "星穹铁道管理器无法连接到游戏；不会执行任何游戏操作。",
                            "The Star Rail manager could not connect to the game; no game action was performed.",
                        ),
                        error,
                    ));
                },
            };
            scanner = scanner.with_observer(manager_scan_observer(progress.clone()));
            let inventory = match scanner.scan_manager_inventory() {
                Ok(inventory) => inventory,
                Err(_) if user_aborted(&cancel) => return Ok(stopped(TaskKind::Manager)),
                Err(error) => {
                    return Err(hsr_ui_error(
                        UiText::new(
                            "无法证明遗器库存已完整扫描；不会执行任何游戏操作。",
                            "A complete Relic inventory scan could not be proven; no game action was performed.",
                        ),
                        error,
                    ));
                },
            };
            if let Some(step) = progress.lock().unwrap().steps.first_mut() {
                step.completed = inventory.len();
                step.state = super::task_progress::StepState::Complete;
            }
            if user_aborted(&cancel) {
                return Ok(stopped(TaskKind::Manager));
            }
            let plan = build_manager_plan(&envelope, &inventory).map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "无法生成安全的管理器预览；不会执行任何游戏操作。",
                        "A safe manager preview could not be built; no game action was performed.",
                    ),
                    error,
                )
            })?;
            if user_aborted(&cancel) {
                return Ok(stopped(TaskKind::Manager));
            }
            let fresh_preview = build_manager_preview(plan, identity, false)?;
            let actionable = publish_manager_preview(&preview, fresh_preview);
            Ok(UiText::new(
                format!("精确预览已就绪：{} 项可授权变更。", actionable),
                format!("Exact preview ready: {actionable} authorizable change(s)."),
            ))
        },
    )
}

pub fn spawn_manager_apply(
    settings: &StarRailSettings,
    status: Arc<Mutex<TaskStatus>>,
    preview: Arc<Mutex<Option<ManagerPreview>>>,
    authorized_scopes: BTreeSet<MutationScope>,
    progress: Arc<Mutex<super::task_progress::TaskProgress>>,
) -> TaskHandle {
    let settings = settings.clone();
    let expected_identity = settings_identity(&settings);
    let status_for_progress = status.clone();
    {
        let mut track = progress.lock().unwrap();
        if let Some(step) = track.steps.iter_mut().find(|s| s.key == "gear") {
            step.completed = 0;
            step.total = None;
            step.state = super::task_progress::StepState::Pending;
        } else {
            track.steps.insert(
                0,
                super::task_progress::Step::new("gear", "重扫遗器库存", "Rescan Relic inventory"),
            );
        }
    }
    worker::spawn_cancellable_task(
        TaskKind::Manager,
        LogSource::Manager,
        status,
        UiText::new(
            "正在重新验证预览并应用已授权变更...",
            "Revalidating the preview and applying authorized changes...",
        ),
        UiText::new(
            "正在安全停止星穹铁道管理器...",
            "Stopping the Star Rail manager safely...",
        ),
        move |cancel| {
            ensure_ocr_runtime()?;
            let reviewed = match preview.lock() {
                Ok(slot) => slot.clone(),
                Err(poisoned) => {
                    preview.clear_poison();
                    poisoned.into_inner().clone()
                },
            }
            .ok_or_else(|| {
                UiError::from_message(
                    UiText::new(
                        "精确预览已失效。请重新预览后再确认；不会执行任何游戏操作。",
                        "The exact preview is no longer valid. Preview again before confirming; no game action was performed.",
                    ),
                    "manager apply started without a retained preview",
                )
            })?;
            if reviewed.settings_identity != expected_identity {
                return Err(UiError::from_message(
                    UiText::new(
                        "预览后设置发生了变化。请重新预览；不会执行任何游戏操作。",
                        "Settings changed after the preview. Preview again; no game action was performed.",
                    ),
                    "manager settings identity differs from reviewed preview",
                ));
            }

            let references = load_references().map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "无法加载星穹铁道游戏数据。请重新下载最新版本的程序后重试。",
                        "Star Rail game data could not be loaded. Download the latest app build and retry.",
                    ),
                    error,
                )
            })?;
            let envelope =
                load_manager_instructions(Path::new(settings.manager_instructions_path.trim()))?;
            validate_manager_envelope_reference(&envelope, &references).map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "管理指令与当前参考数据不一致；不会执行任何游戏操作。",
                        "The manager instructions do not match the current reference data; no game action was performed.",
                    ),
                    error,
                )
            })?;
            if user_aborted(&cancel) {
                return Err(UiError::from_message(
                    UiText::new(
                        "已在进入游戏控制阶段前停止管理操作；未执行任何变更。",
                        "The manager operation stopped before game control began; no changes were made.",
                    ),
                    "manager apply cancelled by user before controller lease acquisition",
                ));
            }
            let controller_lease = HsrControllerLease::try_acquire().map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "另一个星穹铁道操作正在使用游戏控制器；不会执行任何游戏操作。",
                        "Another Star Rail operation is using the game controller; no game action was performed.",
                    ),
                    error,
                )
            })?;
            let journal = AppendOnlyJsonJournalStore::new(PathBuf::from(
                settings.manager_journal_path.trim(),
            ))
            .try_acquire_apply_lease_with_controller(controller_lease)
            .map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "无法取得管理恢复日志的独占权限；不会执行任何游戏操作。",
                        "Exclusive access to the manager recovery journal could not be acquired; no game action was performed.",
                    ),
                    error,
                )
            })?;
            let authorization =
                ApplyAuthorization::new(reviewed.plan.digest.clone(), authorized_scopes);
            let config = manager_scan_config(&settings)?;
            let mut scanner = HsrScanner::live_with_cancel(references, config, cancel.clone()).map_err(
                |error| {
                    hsr_ui_error(
                        UiText::new(
                            "星穹铁道管理器无法连接到游戏；不会执行任何游戏操作。",
                            "The Star Rail manager could not connect to the game; no game action was performed.",
                        ),
                        error,
                    )
                },
            )?;
            scanner = scanner.with_observer(manager_scan_observer(progress.clone()));
            let mut journal = super::hsr_manager_progress::TrackedJournal {
                inner: journal,
                progress: progress.clone(),
                status: status_for_progress,
                cancel,
            };
            let expected_digest = reviewed.plan.digest.clone();
            let report = apply_manager_envelope(
                &envelope,
                &authorization,
                &mut scanner,
                &mut journal,
                |scanner| scanner.scan_manager_inventory(),
                |plan| {
                    if plan.digest != expected_digest {
                        return Err(HsrError::new(
                            "HSR_MANAGER_UI_PREVIEW_CHANGED",
                            hsr_scanner::LocalizedText::new(
                                "最新库存生成了不同的管理预览；不会执行任何游戏操作。",
                                "The latest inventory produced a different manager preview; no game action was performed.",
                            ),
                            "fresh manager plan digest differs from the UI-reviewed digest",
                        ));
                    }
                    if let Some(step) = progress.lock().unwrap().steps.iter_mut().find(|s| s.key == "gear") {
                        step.state = super::task_progress::StepState::Complete;
                    }
                    Ok(())
                },
            )
            .map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "已授权的管理操作未能安全完成。请查看恢复日志并复制完整错误。",
                        "The authorized manager operation could not finish safely. Review the recovery journal and copy the full error.",
                    ),
                    error,
                )
            })?;
            match preview.lock() {
                Ok(mut slot) => *slot = None,
                Err(poisoned) => {
                    preview.clear_poison();
                    *poisoned.into_inner() = None;
                },
            }
            if report.needs_review_actions > 0 {
                Err(UiError::from_message(
                    UiText::new(
                        "管理操作已停止，仍有操作需要人工复核。请保留恢复日志，不要重复点击。",
                        "The manager stopped with actions still requiring manual review. Keep the recovery journal and do not repeat the clicks.",
                    ),
                    format!(
                        "verifiedActions={}; needsReviewActions={}; deviceToggles={}; journal={}",
                        report.verified_actions,
                        report.needs_review_actions,
                        report.device_toggles,
                        settings.manager_journal_path
                    ),
                ))
            } else {
                let journal_archive = archive_completed_manager_journal(Path::new(
                    settings.manager_journal_path.trim(),
                ))?;
                Ok(UiText::new(
                    format!(
                        "管理操作完成：{} 项已验证；设备切换 {} 次。{}",
                        report.verified_actions,
                        report.device_toggles,
                        journal_archive
                            .as_ref()
                            .map_or_else(String::new, |path| format!(
                                "恢复日志已归档至 {}。",
                                path.display()
                            ))
                    ),
                    format!(
                        "Manager operation complete: {} verified; {} device toggle(s). {}",
                        report.verified_actions,
                        report.device_toggles,
                        journal_archive
                            .as_ref()
                            .map_or_else(String::new, |path| format!(
                                "Recovery journal archived to {}.",
                                path.display()
                            ))
                    ),
                ))
            }
        },
    )
}
#[cfg(test)]
mod feedback_tests {
    use super::*;
    #[test]
    fn actionable_backend_hint_is_primary_and_context_is_retained() {
        let error = HsrError::new(
            "HSR-TEST",
            hsr_scanner::localization::LocalizedText::new(
                "请切回游戏。",
                "Switch back to the game.",
            ),
            "FOCUS_LOST_MARKER",
        );
        let failure = hsr_ui_error(UiText::new("扫描未完成", "Scan did not finish"), error);
        assert_eq!(
            failure.hint_text(super::super::state::Lang::En),
            "Switch back to the game."
        );
        assert_eq!(
            failure.hint_text(super::super::state::Lang::Zh),
            "请切回游戏。"
        );
        assert!(failure
            .copy_text(super::super::state::Lang::En)
            .contains("FOCUS_LOST_MARKER"));
    }
}
