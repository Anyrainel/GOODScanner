use std::sync::{Arc, Mutex};

use eframe::egui;

use super::state::{self, Lang, TaskKind, UiError, UiText};
use super::widgets;
use super::worker;

use genshin_scanner::capture::monitor::{CaptureCommand, CaptureState};
use genshin_scanner::capture::player_data::CaptureExportSettings;
use genshin_scanner::scanner::common::models::GoodExport;

const CAPTURE_EXPORT_PREFIX: &str = "genshin_export_";
const CAPTURE_EXPORT_SUFFIX: &str = ".json";

/// Handle to the capture monitor running on a background tokio runtime.
pub struct CaptureHandle {
    _thread: std::thread::JoinHandle<()>,
    cmd_tx: Mutex<Option<tokio::sync::mpsc::UnboundedSender<CaptureCommand>>>,
    native_crash: Arc<worker::NativeCrashState>,
    native_failure: Mutex<Option<UiError>>,
}

impl CaptureHandle {
    pub fn send(&self, cmd: CaptureCommand) {
        if let Ok(cmd_tx) = self.cmd_tx.lock() {
            if let Some(cmd_tx) = &*cmd_tx {
                let _ = cmd_tx.send(cmd);
            }
        }
    }

    pub fn close(&self) {
        match self.cmd_tx.lock() {
            Ok(mut cmd_tx) => {
                cmd_tx.take();
            },
            Err(poisoned) => {
                self.cmd_tx.clear_poison();
                poisoned.into_inner().take();
            },
        }
    }

    fn surface_native_failure(&self, phase: Option<UiText>) {
        if !self.native_crash.has_occurred() {
            return;
        }
        let Some(exception) = self.native_crash.claim_exception(TaskKind::Capture, phase) else {
            return;
        };
        worker::deactivate_native_crash(&self.native_crash);
        self.send(CaptureCommand::StopCapture);
        self.close();
        let error = UiError::native_exception(exception);
        match self.native_failure.lock() {
            Ok(mut slot) => *slot = Some(error.clone()),
            Err(poisoned) => {
                self.native_failure.clear_poison();
                *poisoned.into_inner() = Some(error.clone());
            },
        }
        let lang = if yas::lang::is_en() {
            Lang::En
        } else {
            Lang::Zh
        };
        log::error!(target: yas::lang::LOCALIZED_LOG_TARGET, "{}", error.copy_text(lang));
    }

    pub fn is_finished(&self) -> bool {
        self.surface_native_failure(None);
        self.has_native_failure() || self._thread.is_finished()
    }

    fn has_native_failure(&self) -> bool {
        match self.native_failure.lock() {
            Ok(slot) => slot.is_some(),
            Err(poisoned) => poisoned.into_inner().is_some(),
        }
    }

    fn native_failure(&self, phase: Option<UiText>) -> Option<UiError> {
        self.surface_native_failure(phase);
        match self.native_failure.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

/// Pending export result (polled each frame).
struct PendingExport {
    rx: tokio::sync::oneshot::Receiver<anyhow::Result<GoodExport>>,
}

/// Lifecycle phases for the capture tab.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// Nothing running yet. Show Start button.
    Idle,
    /// Background thread initializing (downloading data cache, loading keys).
    Initializing,
    /// Capture active, waiting for game packets.
    Waiting,
    /// Stop/close requested; keep the handle until its thread really exits.
    Stopping,
    /// All data received — auto-exporting.
    Exporting,
    /// Done — file written.
    Done { summary: UiText, path: String },
    /// Something failed.
    Failed(UiError),
}

/// State specific to the capture tab (lives in GuiApp, not AppState).
pub struct CaptureTabState {
    pub handle: Option<CaptureHandle>,
    pub capture_state: Arc<Mutex<CaptureState>>,
    phase: Phase,
    pending_export: Option<PendingExport>,

    // Export settings
    pub include_characters: bool,
    pub include_weapons: bool,
    pub include_artifacts: bool,
    pub include_achievements: bool,
    pub output_dir: String,

    // Advanced
    pub dump_packets: bool,
    pub only_keep_latest_dump: bool,
    pub data_cache_refresh: state::RefreshState,
}

impl CaptureTabState {
    pub fn new(output_dir: String) -> Self {
        Self {
            handle: None,
            capture_state: Arc::new(Mutex::new(CaptureState::default())),
            phase: Phase::Idle,
            pending_export: None,
            include_characters: true,
            include_weapons: true,
            include_artifacts: true,
            include_achievements: true,
            output_dir,
            dump_packets: false,
            only_keep_latest_dump: false,
            data_cache_refresh: state::RefreshState::Idle,
        }
    }

    pub fn from_config(output_dir: String, config: &genshin_scanner::cli::GoodUserConfig) -> Self {
        let mut state = Self::new(output_dir);
        state.include_characters = config.capture_include_characters;
        state.include_weapons = config.capture_include_weapons;
        state.include_artifacts = config.capture_include_artifacts;
        state.include_achievements = config.capture_include_achievements;
        state.dump_packets = config.capture_dump_packets;
        state.only_keep_latest_dump = config.capture_only_keep_latest_export;
        state
    }

    pub fn sync_to_config(&self, config: &mut genshin_scanner::cli::GoodUserConfig) {
        config.capture_include_characters = self.include_characters;
        config.capture_include_weapons = self.include_weapons;
        config.capture_include_artifacts = self.include_artifacts;
        config.capture_include_achievements = self.include_achievements;
        config.capture_dump_packets = self.dump_packets;
        config.capture_only_keep_latest_export = self.only_keep_latest_dump;
    }

    pub fn is_busy(&self) -> bool {
        matches!(
            self.phase,
            Phase::Initializing | Phase::Waiting | Phase::Stopping | Phase::Exporting
        )
    }

    pub fn requires_restart(&self) -> bool {
        self.native_failure().is_some()
    }

    fn native_failure(&self) -> Option<UiError> {
        let phase = match &self.phase {
            Phase::Initializing => Some(UiText::new("正在初始化抓包器", "Initializing capture")),
            Phase::Waiting => Some(UiText::new("正在读取游戏数据", "Reading game data")),
            Phase::Stopping => Some(UiText::new("正在停止抓包器", "Stopping capture")),
            Phase::Exporting => Some(UiText::new("正在导出抓包数据", "Exporting captured data")),
            _ => None,
        };
        self.handle
            .as_ref()
            .and_then(|handle| handle.native_failure(phase))
    }
}

/// Spawn the capture monitor on a background thread with a tokio runtime.
fn spawn_capture(
    capture_state: Arc<Mutex<CaptureState>>,
    cmd_tx_out: &mut Option<tokio::sync::mpsc::UnboundedSender<CaptureCommand>>,
    dump_packets: bool,
    include_achievements: bool,
    native_crash: Arc<worker::NativeCrashState>,
) -> Result<std::thread::JoinHandle<()>, UiError> {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
    let ui_cmd_tx = cmd_tx.clone();

    let state = capture_state.clone();

    let thread = std::thread::Builder::new()
        .name("capture-monitor".to_owned())
        .spawn(move || {
        let native_guard_active = worker::activate_native_crash(&native_crash);
        if !native_guard_active {
            if let Ok(mut state) = state.lock() {
                state.error =
                    Some("native crash boundary is still owned by another task".to_string());
            }
            return;
        }
        let native_thread_context =
            native_guard_active.then(yas::native_crash::inherit_current_task);
        let state_for_crash = state.clone();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .on_thread_start(|| {
                    // Tokio owns these threads for this capture runtime. The
                    // registry is cleared when the task ends or crashes.
                    std::mem::forget(yas::native_crash::inherit_current_task());
                })
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    yas::log_error!(
                        "抓包运行环境无法启动。请检查系统资源，然后重试。完整错误详情: {:#}",
                        "The capture runtime could not start. Check available system resources, then retry. Full error details: {:#}",
                        e,
                    );
                    if let Ok(mut s) = state.lock() {
                        s.error = Some(format!("{:#}", e));
                    }
                    return;
                },
            };

            rt.block_on(async {
                let monitor = match genshin_scanner::capture::monitor::CaptureMonitor::new(
                    state.clone(),
                    dump_packets,
                ) {
                    Ok(m) => m,
                    Err(e) => {
                        yas::log_error!(
                            "抓包器无法初始化。请检查下方完整错误。完整错误详情: {:#}",
                            "Capture could not initialize. Check the complete error below. Full error details: {:#}",
                            e,
                        );
                        if let Ok(mut s) = state.lock() {
                            s.error = Some(format!("{:#}", e));
                        }
                        return;
                    },
                };

                // Initialization succeeded — immediately start capture
                let _ = cmd_tx.send(CaptureCommand::StartCapture {
                    include_achievements,
                });
                // The UI's CaptureHandle owns the long-lived sender. Dropping
                // this thread-local clone lets monitor.run observe channel
                // closure when the UI stops, retries, or discards the task.
                drop(cmd_tx);

                monitor.run(cmd_rx).await;
            });
        }));

        if let Err(panic_info) = result {
            let failure = UiError::from_panic(
                UiText::new(
                    "抓包任务因意外的内部错误而停止。",
                    "The capture task stopped because of an unexpected internal error.",
                ),
                panic_info.as_ref(),
            );
            let lang = if yas::lang::is_en() {
                Lang::En
            } else {
                Lang::Zh
            };
            let msg = failure.technical_details(lang);
            log::error!(target: yas::lang::LOCALIZED_LOG_TARGET, "{}", failure.copy_text(lang));
            if let Ok(mut s) = state_for_crash.lock() {
                s.error = Some(msg);
            }
        }
        drop(native_thread_context);
        if native_guard_active {
            worker::deactivate_native_crash(&native_crash);
        }
        })
        .map_err(|error| {
            UiError::from_error(
                UiText::new(
                    "抓包器后台任务无法启动。请检查系统资源，然后重试。",
                    "The capture background task could not start. Check available system resources, then retry.",
                ),
                error,
            )
        })?;
    *cmd_tx_out = Some(ui_cmd_tx);
    Ok(thread)
}

pub fn show(
    ui: &mut egui::Ui,
    l: Lang,
    tab: &mut CaptureTabState,
    game_busy: bool,
    restart_required: bool,
) {
    // --- Phase transitions driven by shared state ---
    update_phase(tab, l);

    let is_busy = tab.is_busy();

    // === Action bar (always visible at top) ===
    ui.add_space(4.0);
    action_bar(ui, l, tab, game_busy, restart_required);
    if restart_required {
        return;
    }
    if !is_busy {
        ui.colored_label(
            egui::Color32::from_rgb(120, 120, 120),
            l.t(
                "通过抓包获取游戏数据（角色/武器/圣遗物/成就），需管理员权限。",
                "Capture game data (characters/weapons/artifacts/achievements) via packet sniffing. Requires admin.",
            ),
        );
    }
    ui.colored_label(
        egui::Color32::from_rgb(80, 150, 220),
        l.t(
            "GOODCapture 现已包含 GOODScanner，今后只需使用一个程序即可。",
            "GOODScanner is now included in GOODCapture, so you only need one program going forward.",
        ),
    );
    ui.add_space(4.0);
    ui.separator();

    // === Scrollable config area ===
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);

            // === Export Settings ===
            egui::CollapsingHeader::new(l.t("导出设置", "Export Settings"))
                .default_open(true)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!is_busy, |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut tab.include_characters, l.t("角色", "Characters"));
                            ui.add_space(12.0);
                            ui.checkbox(&mut tab.include_weapons, l.t("武器", "Weapons"));
                            ui.add_space(12.0);
                            ui.checkbox(&mut tab.include_artifacts, l.t("圣遗物", "Artifacts"));
                            ui.add_space(12.0);
                            ui.checkbox(&mut tab.include_achievements, l.t("成就", "Achievements"));
                        });
                    });
                });

            // === Advanced settings ===
            egui::CollapsingHeader::new(l.t("高级设置", "Advanced"))
                .default_open(false)
                .show(ui, |ui| {
                    ui.checkbox(
                        &mut tab.dump_packets,
                        l.t(
                            "保存所有数据包 → debug_capture/",
                            "Dump all decrypted packets → debug_capture/",
                        ),
                    );
                    ui.checkbox(
                        &mut tab.only_keep_latest_dump,
                        l.t("仅保留最新导出", "Only keep latest export"),
                    );

                    tab.data_cache_refresh.poll();
                    widgets::game_data_refresh_control(
                        ui,
                        l,
                        &mut tab.data_cache_refresh,
                        UiText::new(
                            "无法刷新抓包器使用的游戏数据。请检查网络连接，然后重试。",
                            "Capture's game data could not be refreshed. Check the network connection, then retry.",
                        ),
                        genshin_scanner::capture::data_cache::force_refresh,
                    );
                });

            // === Help / FAQ ===
            egui::CollapsingHeader::new(l.t("使用说明", "How to use"))
                .default_open(false)
                .show(ui, |ui| {
                    let steps = match l {
                        Lang::Zh => &[
                            "1. 点击「开始抓包」后，软件开始监听网络数据包。",
                            "2. 如果游戏已在运行，请关闭并重新启动，登录进入游戏（过门）。",
                            "3. 软件会在收到角色和物品数据后自动停止并导出 JSON 文件。",
                            "4. 导出的文件可直接导入到 ggartifact.com 等工具中使用。",
                        ] as &[&str],
                        Lang::En => &[
                            "1. Click 'Start Capture' to begin listening for network packets.",
                            "2. If the game is already running, close it, relaunch, and log in (enter door).",
                            "3. Once character and item data are received, capture stops automatically and exports a JSON file.",
                            "4. The exported file can be imported directly into ggartifact.com and similar tools.",
                        ],
                    };
                    for step in steps {
                        ui.label(*step);
                    }
                });
        });
}

/// Top action bar: start/stop button + inline status.
fn action_bar(
    ui: &mut egui::Ui,
    l: Lang,
    tab: &mut CaptureTabState,
    game_busy: bool,
    restart_required: bool,
) {
    if restart_required {
        if let Some(error) = tab.native_failure() {
            widgets::error_card(ui, l, &error);
        } else {
            ui.colored_label(
                egui::Color32::from_rgb(220, 90, 90),
                l.t(
                    "扫描器或管理器发生了底层崩溃。请先复制其中的完整错误，然后重启本程序。",
                    "The scanner or manager had a low-level crash. Copy its full error, then restart this application.",
                ),
            );
        }
        ui.add_enabled(
            false,
            egui::Button::new(l.t("需重启程序", "Restart required")),
        );
        return;
    }

    match &tab.phase {
        Phase::Idle => {
            if game_busy {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 200, 50),
                    l.t(
                        "其他任务正在运行，请等待完成",
                        "Another task is running. Please wait for it to finish.",
                    ),
                );
            }

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !game_busy,
                        egui::Button::new(l.t("▶ 开始抓包", "▶ Start Capture")),
                    )
                    .clicked()
                {
                    if let Err(e) = super::privilege::ensure_admin_for_action() {
                        tab.phase = Phase::Failed(UiError::from_anyhow(
                            UiText::new(
                                "抓包器需要管理员权限才能读取游戏网络数据。请以管理员身份重新启动程序。",
                                "Capture needs administrator access to read game network data. Restart the application as administrator.",
                            ),
                            &e,
                        ));
                    } else {
                        tab.capture_state = Arc::new(Mutex::new(CaptureState::default()));
                        let mut cmd_tx = None;
                        let native_crash = Arc::new(worker::NativeCrashState::new());
                        match spawn_capture(
                            tab.capture_state.clone(),
                            &mut cmd_tx,
                            tab.dump_packets,
                            tab.include_achievements,
                            native_crash.clone(),
                        ) {
                            Ok(thread) => {
                                tab.handle = Some(CaptureHandle {
                                    _thread: thread,
                                    cmd_tx: Mutex::new(cmd_tx),
                                    native_crash,
                                    native_failure: Mutex::new(None),
                                });
                                tab.phase = Phase::Initializing;
                            },
                            Err(error) => {
                                tab.handle = None;
                                tab.phase = Phase::Failed(error);
                            },
                        }
                    }
                }
            });
        },

        Phase::Initializing => {
            ui.horizontal(|ui| {
                if ui.button(l.t("⏹ 停止抓包", "⏹ Stop Capture")).clicked() {
                    if let Some(ref mut h) = tab.handle {
                        h.send(CaptureCommand::StopCapture);
                        h.close();
                    }
                    tab.phase = Phase::Stopping;
                }
                ui.spinner();
                ui.label(l.t(
                    "正在初始化（下载数据缓存）...",
                    "Initializing (downloading data cache)...",
                ));
            });
        },

        Phase::Waiting => {
            ui.horizontal(|ui| {
                if ui.button(l.t("⏹ 停止抓包", "⏹ Stop Capture")).clicked() {
                    if let Some(ref mut h) = tab.handle {
                        h.send(CaptureCommand::StopCapture);
                        h.close();
                    }
                    tab.phase = Phase::Stopping;
                }
                ui.colored_label(
                    egui::Color32::from_rgb(100, 200, 100),
                    l.t("● 正在等待游戏数据...", "● Waiting for game data..."),
                );
            });

            ui.colored_label(
                egui::Color32::from_rgb(120, 120, 120),
                l.t(
                    "请关闭游戏并重新启动，登录（过门）。",
                    "Please close the game, relaunch, and log in (enter door).",
                ),
            );

            // Show partial progress
            if let Ok(cs) = tab.capture_state.try_lock() {
                if cs.has_characters || cs.has_items || cs.has_achievements {
                    let mut parts = Vec::new();
                    if cs.has_characters {
                        parts.push(match l {
                            Lang::Zh => format!("角色: {}", cs.character_count),
                            Lang::En => format!("Characters: {}", cs.character_count),
                        });
                    }
                    if cs.has_items {
                        parts.push(match l {
                            Lang::Zh => {
                                format!("武器: {}, 圣遗物: {}", cs.weapon_count, cs.artifact_count)
                            },
                            Lang::En => format!(
                                "Weapons: {}, Artifacts: {}",
                                cs.weapon_count, cs.artifact_count
                            ),
                        });
                    }
                    if cs.has_achievements {
                        parts.push(match l {
                            Lang::Zh => format!("成就: {}", cs.achievement_count),
                            Lang::En => format!("Achievements: {}", cs.achievement_count),
                        });
                    }
                    ui.colored_label(egui::Color32::from_rgb(100, 200, 100), parts.join("  |  "));

                    let mut missing = Vec::new();
                    if !cs.has_characters {
                        missing.push(l.t("角色", "characters"));
                    }
                    if !cs.has_items {
                        missing.push(l.t("物品", "items"));
                    }
                    if tab.include_achievements && !cs.has_achievements {
                        missing.push(l.t("成就", "achievements"));
                    }
                    if !missing.is_empty() {
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 200, 50),
                            match l {
                                Lang::Zh => format!("等待{}数据...", missing.join("、")),
                                Lang::En => format!("Waiting for {} data...", missing.join(", ")),
                            },
                        );
                    }
                }
            }
        },

        Phase::Exporting => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(l.t("正在导出...", "Exporting..."));
            });
        },

        Phase::Stopping => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(l.t("正在停止抓包...", "Stopping capture..."));
            });
        },

        Phase::Done { summary, path } => {
            let summary = summary.clone();
            let path = path.clone();
            ui.horizontal(|ui| {
                if ui.button(l.t("↻ 重新抓包", "↻ Recapture")).clicked() {
                    if let Some(ref mut handle) = tab.handle {
                        handle.close();
                        tab.phase = Phase::Stopping;
                    } else {
                        tab.phase = Phase::Idle;
                    }
                }
                ui.colored_label(egui::Color32::from_rgb(100, 200, 100), summary.text(l));
            });
            ui.label(egui::RichText::new(format!("→ {}", path)).size(11.0).weak());
        },

        Phase::Failed(error) => {
            let error = error.clone();
            ui.horizontal(|ui| {
                if ui.button(l.t("↻ 重试", "↻ Retry")).clicked() {
                    if let Some(ref mut handle) = tab.handle {
                        handle.close();
                        tab.phase = Phase::Stopping;
                    } else {
                        tab.phase = Phase::Idle;
                    }
                }
            });
            widgets::error_card(ui, l, &error);
        },
    }
}

/// Drive phase transitions based on shared capture state.
fn update_phase(tab: &mut CaptureTabState, _l: Lang) {
    if let Some(error) = tab.native_failure() {
        tab.phase = Phase::Failed(error);
        return;
    }

    if tab.phase == Phase::Stopping {
        if tab.handle.as_ref().map_or(true, CaptureHandle::is_finished) {
            tab.handle = None;
            tab.phase = Phase::Idle;
        }
        return;
    }

    // Poll pending export
    if let Some(ref mut pending) = tab.pending_export {
        match pending.rx.try_recv() {
            Ok(Ok(export)) => {
                let timestamp = genshin_scanner::cli::chrono_timestamp();
                let filename = format!(
                    "{}{}{}",
                    CAPTURE_EXPORT_PREFIX, timestamp, CAPTURE_EXPORT_SUFFIX
                );
                let path = std::path::Path::new(&tab.output_dir).join(&filename);
                if tab.only_keep_latest_dump {
                    match remove_previous_capture_exports(std::path::Path::new(&tab.output_dir)) {
                        Ok(removed) => {
                            if removed > 0 {
                                yas::log_info!(
                                    "仅保留最新导出：已删除 {} 个旧导出",
                                    "Only keep latest dump: removed {} old export(s)",
                                    removed
                                );
                            }
                        },
                        Err(e) => {
                            tab.phase = Phase::Failed(UiError::from_anyhow(
                                UiText::new(
                                    "无法清理旧导出文件。请检查输出目录是否可写，或关闭正在使用这些文件的程序。",
                                    "Old export files could not be removed. Check that the output folder is writable and that no other application is using those files.",
                                ),
                                &e,
                            ));
                            tab.pending_export = None;
                            return;
                        },
                    }
                }
                match serde_json::to_string_pretty(&export) {
                    Ok(json) => match std::fs::write(&path, &json) {
                        Ok(_) => {
                            let cc = export.characters.as_ref().map_or(0, |v| v.len());
                            let wc = export.weapons.as_ref().map_or(0, |v| v.len());
                            let ac = export.artifacts.as_ref().map_or(0, |v| v.len());
                            let hc = export.achievements.as_ref().map_or(0, |v| v.len());
                            let summary = UiText::new(
                                format!(
                                    "已导出: {} 角色, {} 武器, {} 圣遗物, {} 成就",
                                    cc, wc, ac, hc
                                ),
                                format!(
                                    "Exported: {} characters, {} weapons, {} artifacts, {} achievements",
                                    cc, wc, ac, hc
                                ),
                            );
                            yas::log_info!("{} → {}", "{} → {}", summary, path.display());
                            tab.phase = Phase::Done {
                                summary,
                                path: path.display().to_string(),
                            };
                        },
                        Err(e) => {
                            tab.phase = Phase::Failed(UiError::from_error(
                                UiText::new(
                                    "导出文件无法写入。请检查输出目录、可用磁盘空间和文件权限。",
                                    "The export file could not be written. Check the output folder, available disk space, and file permissions.",
                                ),
                                e,
                            ));
                        },
                    },
                    Err(e) => {
                        tab.phase = Phase::Failed(UiError::from_error(
                            UiText::new(
                                "抓取的数据无法转换为导出文件。请复制完整错误并报告此问题。",
                                "The captured data could not be converted into an export file. Copy the full error and report this problem.",
                            ),
                            e,
                        ));
                    },
                }
                tab.pending_export = None;
                return;
            },
            Ok(Err(e)) => {
                tab.phase = Phase::Failed(UiError::from_anyhow(
                    UiText::new(
                        "抓包数据导出未能完成。下方完整错误包含底层原因。",
                        "The captured data could not be exported. The full error below contains the underlying cause.",
                    ),
                    &e,
                ));
                tab.pending_export = None;
                return;
            },
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                return; // still waiting
            },
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                tab.phase = Phase::Failed(UiError::from_message(
                    UiText::new(
                        "导出任务意外停止，未能返回结果。请重试；若再次发生，请复制完整错误并报告问题。",
                        "The export task stopped unexpectedly without returning a result. Retry; if it happens again, copy the full error and report the problem.",
                    ),
                    "tokio oneshot export result channel closed before sending a result",
                ));
                tab.pending_export = None;
                return;
            },
        }
    }

    // Check for errors from background thread
    if matches!(tab.phase, Phase::Initializing | Phase::Waiting) {
        if let Ok(cs) = tab.capture_state.try_lock() {
            if let Some(ref err) = cs.error {
                tab.phase = Phase::Failed(UiError::from_message(
                    UiText::new(
                        "抓包器在启动或读取游戏数据时停止。下方完整错误包含底层原因。",
                        "Capture stopped while starting or reading game data. The full error below contains the underlying cause.",
                    ),
                    err.clone(),
                ));
                return;
            }
        }

        // Check if monitor thread died unexpectedly
        if tab.handle.as_ref().map_or(false, |h| h.is_finished()) {
            let has_error = tab
                .capture_state
                .try_lock()
                .map_or(false, |s| s.error.is_some());
            if !has_error {
                tab.phase = Phase::Failed(UiError::from_message(
                    UiText::new(
                        "抓包任务意外停止，且没有返回结果。请重试；若再次发生，请复制完整错误并报告问题。",
                        "The capture task stopped unexpectedly without returning a result. Retry; if it happens again, copy the full error and report the problem.",
                    ),
                    "capture worker thread exited without reporting an error",
                ));
            }
            return;
        }
    }

    // Transition: Initializing → Waiting (when capture starts)
    if tab.phase == Phase::Initializing {
        if tab.capture_state.try_lock().map_or(false, |s| s.capturing) {
            tab.phase = Phase::Waiting;
        }
    }

    // Transition: Waiting → auto-export (when capture auto-stopped with complete data)
    if tab.phase == Phase::Waiting {
        if tab.capture_state.try_lock().map_or(false, |s| s.complete) {
            // Automatically trigger export
            let settings = CaptureExportSettings {
                include_characters: tab.include_characters,
                include_weapons: tab.include_weapons,
                include_artifacts: tab.include_artifacts,
                include_achievements: tab.include_achievements,
                ..Default::default()
            };
            let (tx, rx) = tokio::sync::oneshot::channel();
            if let Some(ref h) = tab.handle {
                h.send(CaptureCommand::Export {
                    settings,
                    reply: tx,
                });
                tab.pending_export = Some(PendingExport { rx });
                tab.phase = Phase::Exporting;
            }
        }
    }
}

fn remove_previous_capture_exports(output_dir: &std::path::Path) -> anyhow::Result<usize> {
    let mut removed = 0;
    for entry in std::fs::read_dir(output_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if is_generated_capture_export_filename(file_name) {
            std::fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn is_generated_capture_export_filename(file_name: &str) -> bool {
    if !file_name.starts_with(CAPTURE_EXPORT_PREFIX) || !file_name.ends_with(CAPTURE_EXPORT_SUFFIX)
    {
        return false;
    }

    let timestamp =
        &file_name[CAPTURE_EXPORT_PREFIX.len()..file_name.len() - CAPTURE_EXPORT_SUFFIX.len()];
    is_capture_export_timestamp(timestamp)
}

fn is_capture_export_timestamp(timestamp: &str) -> bool {
    let bytes = timestamp.as_bytes();
    if bytes.len() != 19 {
        return false;
    }
    for (idx, byte) in bytes.iter().enumerate() {
        let expected_separator = matches!(idx, 4 | 7 | 13 | 16);
        if expected_separator {
            if *byte != b'-' {
                return false;
            }
        } else if idx == 10 {
            if *byte != b'_' {
                return false;
            }
        } else if !byte.is_ascii_digit() {
            return false;
        }
    }

    let month = parse_two_digits(&bytes[5..7]);
    let day = parse_two_digits(&bytes[8..10]);
    let hour = parse_two_digits(&bytes[11..13]);
    let minute = parse_two_digits(&bytes[14..16]);
    let second = parse_two_digits(&bytes[17..19]);

    (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour <= 23
        && minute <= 59
        && second <= 59
}

fn parse_two_digits(bytes: &[u8]) -> u8 {
    (bytes[0] - b'0') * 10 + (bytes[1] - b'0')
}

#[cfg(test)]
mod tests {
    use super::is_generated_capture_export_filename;

    #[test]
    fn matches_generated_capture_exports_only() {
        assert!(is_generated_capture_export_filename(
            "genshin_export_2026-04-27_13-45-09.json"
        ));
        assert!(!is_generated_capture_export_filename(
            "genshin_export_2026-04-27_13-45.json"
        ));
        assert!(!is_generated_capture_export_filename(
            "genshin_export_latest.json"
        ));
        assert!(!is_generated_capture_export_filename(
            "genshin_export_2026-13-27_13-45-09.json"
        ));
        assert!(!is_generated_capture_export_filename(
            "good_export_2026-04-27_13-45-09.json"
        ));
    }
}
