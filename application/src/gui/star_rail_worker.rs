use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use hsr_scanner::{
    capture::import_reliquary_archive_file,
    load_embedded_gilore_reference,
    manager::{
        apply_manager_envelope, build_manager_plan, load_manager_recovery_plan,
        validate_manager_envelope_reference, AppendOnlyJsonJournalStore, ApplyAuthorization,
        HsrControllerLease, ManagerInstructionsEnvelope, ManagerJournalStore, ManagerPlan,
        MutationScope,
    },
    pipeline::{build_export, write_export_create_new},
    reference::ReferenceCache,
    scanner::{HsrScanner, ScanConfig, ScanTargets},
    HsrError, Language,
};

use crate::config::StarRailSettings;

use super::{
    star_rail_state::ManagerPreview,
    state::{LogSource, TaskKind, TaskStatus, UiError, UiText},
    worker::{self, TaskHandle},
};

const MAX_MANAGER_INSTRUCTIONS_BYTES: u64 = 16 * 1024 * 1024;

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
    UiError::from_message(hint, error.localized_message(hsr_language()))
}

fn ensure_ocr_runtime() -> Result<(), UiError> {
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
    let mut keys = settings.next_character_key.chars();
    let next_character_key = keys.next().ok_or_else(|| {
        UiError::from_message(
            UiText::new(
                "“下一个角色”按键不能为空。请输入一个按键。",
                "The Next Character key cannot be empty. Enter one key.",
            ),
            "starRail.nextCharacterKey is empty",
        )
    })?;
    if keys.next().is_some() {
        return Err(UiError::from_message(
            UiText::new(
                "“下一个角色”按键只能包含一个字符。",
                "The Next Character key must contain exactly one character.",
            ),
            format!(
                "starRail.nextCharacterKey has {} characters",
                settings.next_character_key.chars().count()
            ),
        ));
    }

    Ok(ScanConfig {
        targets,
        capture_method: settings.capture_method.to_yas(),
        navigation_delay: Duration::from_millis(settings.navigation_delay_ms),
        panel_timeout: Duration::from_millis(settings.panel_timeout_ms),
        max_inventory_items: settings.max_inventory_items,
        max_characters: settings.max_characters,
        expected_characters: (settings.expected_characters > 0)
            .then_some(settings.expected_characters),
        next_character_key,
        ..ScanConfig::default()
    })
}

fn ensure_output_dir(settings: &StarRailSettings) -> Result<PathBuf, UiError> {
    let output_dir = PathBuf::from(settings.output_dir.trim());
    if settings.output_dir.trim().is_empty() {
        return Err(UiError::from_message(
            UiText::new(
                "请选择星穹铁道导出的保存文件夹。",
                "Choose a folder for Star Rail exports.",
            ),
            "starRail.outputDir is empty",
        ));
    }
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

/// All app flows use the bundled reference data without user configuration.
pub fn load_references() -> Result<ReferenceCache, HsrError> {
    let references = load_embedded_gilore_reference()?;
    references.validate_live_complete_profile()?;
    Ok(references)
}

pub fn next_export_path(output_dir: &Path) -> PathBuf {
    output_dir.join(format!(
        "star_rail_export_{}.json",
        genshin_scanner::cli::chrono_timestamp()
    ))
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

pub fn spawn_scan(settings: &StarRailSettings, status: Arc<Mutex<TaskStatus>>) -> TaskHandle {
    let settings = settings.clone();
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
            if user_aborted(&cancel) {
                return Ok(stopped(TaskKind::Scanner));
            }
            ensure_ocr_runtime()?;
            let targets = ScanTargets {
                characters: settings.scan_characters,
                light_cones: settings.scan_light_cones,
                gear: settings.scan_relics_and_ornaments,
            };
            let config = scanner_config(&settings, targets)?;
            let output_dir = ensure_output_dir(&settings)?;
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
                return Ok(stopped(TaskKind::Scanner));
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
            let result = match HsrScanner::live_with_cancel(
                references.clone(),
                config,
                cancel.clone(),
            )
            .and_then(HsrScanner::scan)
            {
                Ok(result) => result,
                Err(_) if user_aborted(&cancel) => return Ok(stopped(TaskKind::Scanner)),
                Err(error) => {
                    return Err(hsr_ui_error(
                        UiText::new(
                            "星穹铁道扫描未能完成。请复制完整错误以搜索或寻求帮助。",
                            "The Star Rail scan could not finish. Copy the full error to search or ask for help.",
                        ),
                        error,
                    ));
                },
            };
            if user_aborted(&cancel) {
                return Ok(stopped(TaskKind::Scanner));
            }
            let export = build_export(result.observations, &references).map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "扫描结果无法转换为 GGStarRail 导入文件。",
                        "The scan result could not be converted into a GGStarRail import file.",
                    ),
                    error,
                )
            })?;
            if user_aborted(&cancel) {
                return Ok(stopped(TaskKind::Scanner));
            }
            let summary = UiText::new(
                format!(
                    "已导出：{} 个角色、{} 个光锥、{} 件隧洞遗器、{} 件位面饰品",
                    export.characters.len(),
                    export.light_cones.len(),
                    export.relics.len(),
                    export.planar_ornaments.len()
                ),
                format!(
                    "Exported {} Characters, {} Light Cones, {} Cavern Relics, and {} Planar Ornaments",
                    export.characters.len(),
                    export.light_cones.len(),
                    export.relics.len(),
                    export.planar_ornaments.len()
                ),
            );
            let path = next_export_path(&output_dir);
            write_export_create_new(&path, &export).map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "星穹铁道导出文件无法写入。请检查输出文件夹、磁盘空间和文件权限。",
                        "The Star Rail export file could not be written. Check the output folder, disk space, and file permissions.",
                    ),
                    error,
                )
            })?;
            Ok(UiText::new(
                format!(
                    "{}。输出：{}",
                    summary.text(super::state::Lang::Zh),
                    path.display()
                ),
                format!(
                    "{}. Output: {}",
                    summary.text(super::state::Lang::En),
                    path.display()
                ),
            ))
        },
    )
}

pub fn spawn_offline_import(
    settings: &StarRailSettings,
    status: Arc<Mutex<TaskStatus>>,
) -> TaskHandle {
    let settings = settings.clone();
    worker::spawn_cancellable_task(
        TaskKind::Scanner,
        LogSource::Scanner,
        status,
        UiText::new(
            "正在导入星穹铁道存档...",
            "Importing the Star Rail archive...",
        ),
        UiText::new("正在停止导入...", "Stopping import..."),
        move |cancel| {
            if user_aborted(&cancel) {
                return Ok(stopped(TaskKind::Scanner));
            }
            let output_dir = ensure_output_dir(&settings)?;
            if settings.offline_import_path.trim().is_empty() {
                return Err(UiError::from_message(
                    UiText::new(
                        "请选择要导入的 Reliquary 或 Fribbels v4 JSON 文件。",
                        "Choose a Reliquary or Fribbels v4 JSON file to import.",
                    ),
                    "starRail.offlineImportPath is empty",
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
                return Ok(stopped(TaskKind::Scanner));
            }
            let imported = import_reliquary_archive_file(
                Path::new(settings.offline_import_path.trim()),
                &references,
            )
            .map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "无法导入这份星穹铁道存档。文件内容未通过安全校验。",
                        "This Star Rail archive could not be imported because it did not pass validation.",
                    ),
                    error,
                )
            })?;
            if user_aborted(&cancel) {
                return Ok(stopped(TaskKind::Scanner));
            }
            let export = build_export(imported.into_observations(), &references).map_err(
                |error| {
                    hsr_ui_error(
                        UiText::new(
                            "导入内容无法转换为 GGStarRail 导入文件。",
                            "The imported data could not be converted into a GGStarRail import file.",
                        ),
                        error,
                    )
                },
            )?;
            if user_aborted(&cancel) {
                return Ok(stopped(TaskKind::Scanner));
            }
            let counts = (
                export.characters.len(),
                export.light_cones.len(),
                export.relics.len(),
                export.planar_ornaments.len(),
            );
            let path = next_export_path(&output_dir);
            write_export_create_new(&path, &export).map_err(|error| {
                hsr_ui_error(
                    UiText::new(
                        "星穹铁道导出文件无法写入。请检查输出文件夹、磁盘空间和文件权限。",
                        "The Star Rail export file could not be written. Check the output folder, disk space, and file permissions.",
                    ),
                    error,
                )
            })?;
            Ok(UiText::new(
                format!(
                    "导入并导出完成：{} 个角色、{} 个光锥、{} 件隧洞遗器、{} 件位面饰品。输出：{}",
                    counts.0,
                    counts.1,
                    counts.2,
                    counts.3,
                    path.display()
                ),
                format!(
                    "Import and export complete: {} Characters, {} Light Cones, {} Cavern Relics, and {} Planar Ornaments. Output: {}",
                    counts.0,
                    counts.1,
                    counts.2,
                    counts.3,
                    path.display()
                ),
            ))
        },
    )
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
    scanner_config(
        settings,
        ScanTargets {
            characters: false,
            light_cones: false,
            gear: true,
        },
    )
}

pub fn spawn_manager_preview(
    settings: &StarRailSettings,
    status: Arc<Mutex<TaskStatus>>,
    preview: Arc<Mutex<Option<ManagerPreview>>>,
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
) -> TaskHandle {
    let settings = settings.clone();
    let expected_identity = settings_identity(&settings);
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
            let mut journal = AppendOnlyJsonJournalStore::new(PathBuf::from(
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
            let mut scanner = HsrScanner::live_with_cancel(references, config, cancel).map_err(
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
