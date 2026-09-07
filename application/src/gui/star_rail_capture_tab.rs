use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
};

use eframe::egui;
use hsr_scanner::{
    packet_capture::{
        CaptureTargets, HsrCaptureCommand, HsrCaptureMonitor, HsrCaptureState, HSR_CAPTURE_REVISION,
    },
    pipeline::{build_achievement_snapshot, build_export, build_export_with_achievements},
    reference::ReferenceCache,
    HsrError, LocalizedText, ValidatedObservationSnapshot,
};

use crate::config::StarRailSettings;

use super::{
    star_rail_worker,
    state::{Lang, TaskKind, UiError, UiText},
    widgets, worker,
};

pub struct StarRailCaptureHandle {
    thread: std::thread::JoinHandle<()>,
    command_tx: Mutex<Option<tokio::sync::mpsc::UnboundedSender<HsrCaptureCommand>>>,
    startup_gate: Arc<CaptureStartupGate>,
    native_crash: Arc<worker::NativeCrashState>,
    native_failure: Mutex<Option<UiError>>,
}

impl StarRailCaptureHandle {
    fn stop(&self) {
        self.startup_gate
            .request_stop(|| match self.command_tx.lock() {
                Ok(mut sender) => {
                    if let Some(sender) = sender.take() {
                        let _ = sender.send(HsrCaptureCommand::StopCapture);
                    }
                },
                Err(poisoned) => {
                    self.command_tx.clear_poison();
                    if let Some(sender) = poisoned.into_inner().take() {
                        let _ = sender.send(HsrCaptureCommand::StopCapture);
                    }
                },
            });
    }

    fn surface_native_failure(&self, phase: Option<UiText>) {
        if !self.native_crash.has_occurred() {
            return;
        }
        let Some(exception) = self.native_crash.claim_exception(TaskKind::Capture, phase) else {
            return;
        };
        worker::deactivate_native_crash(&self.native_crash);
        self.stop();
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

    fn is_finished(&self) -> bool {
        self.surface_native_failure(None);
        self.native_failure().is_some() || self.thread.is_finished()
    }

    fn native_failure(&self) -> Option<UiError> {
        match self.native_failure.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => {
                self.native_failure.clear_poison();
                poisoned.into_inner().clone()
            },
        }
    }
}

/// Serializes the user's Stop request with the worker's first Start command.
/// If Stop wins, the native packet-capture boundary is never opened. If Start
/// wins, Stop is necessarily queued after it and the monitor cancels normally.
#[derive(Default)]
struct CaptureStartupGate {
    serial: Mutex<()>,
    stopped: AtomicBool,
}

impl CaptureStartupGate {
    fn request_stop(&self, stop: impl FnOnce()) {
        let _guard = lock_shared(&self.serial);
        self.stopped.store(true, Ordering::SeqCst);
        stop();
    }

    fn start_if_allowed(&self, start: impl FnOnce() -> bool) -> bool {
        let _guard = lock_shared(&self.serial);
        !self.stopped.load(Ordering::SeqCst) && start()
    }
}

#[cfg(feature = "test-as-invoker")]
#[doc(hidden)]
pub fn stop_before_worker_start_suppresses_start_for_test() -> bool {
    let startup_gate = CaptureStartupGate::default();
    let start_attempted = AtomicBool::new(false);
    startup_gate.request_stop(|| {});
    let start_sent = startup_gate.start_if_allowed(|| {
        start_attempted.store(true, Ordering::SeqCst);
        true
    });
    !start_sent && !start_attempted.load(Ordering::SeqCst)
}

struct PendingExport {
    receiver: mpsc::Receiver<Result<super::star_rail_exports::CaptureFiles, UiError>>,
    thread: std::thread::JoinHandle<()>,
    result: Option<Result<super::star_rail_exports::CaptureFiles, UiError>>,
}

#[derive(Clone, Debug, PartialEq)]
enum CapturePhase {
    Idle,
    Initializing,
    Waiting,
    Stopping,
    Exporting,
    Done { summary: UiText, path: String },
    Failed(UiError),
}

pub struct StarRailCaptureState {
    handle: Option<StarRailCaptureHandle>,
    shared: Arc<Mutex<HsrCaptureState>>,
    references: Arc<Mutex<Option<ReferenceCache>>>,
    pending_export: Option<PendingExport>,
    phase: CapturePhase,
    output_dir: String,
    only_keep_latest_export: bool,
}

impl StarRailCaptureState {
    pub fn new(output_dir: String) -> Self {
        Self {
            handle: None,
            shared: Arc::new(Mutex::new(HsrCaptureState::default())),
            references: Arc::new(Mutex::new(None)),
            pending_export: None,
            phase: CapturePhase::Idle,
            output_dir,
            only_keep_latest_export: false,
        }
    }

    pub fn is_busy(&self) -> bool {
        let phase_busy = matches!(
            self.phase,
            CapturePhase::Initializing
                | CapturePhase::Waiting
                | CapturePhase::Stopping
                | CapturePhase::Exporting
        );
        let monitor_running = self
            .handle
            .as_ref()
            .is_some_and(|handle| !handle.is_finished());
        let export_running = self
            .pending_export
            .as_ref()
            .is_some_and(|pending| !pending.thread.is_finished());
        phase_busy || monitor_running || export_running
    }

    /// Advance capture, stop, and export transitions independently from the
    /// visible tab. The app calls this every frame so switching to another
    /// Star Rail tab cannot strand a completed monitor or its export.
    pub fn tick(&mut self) {
        update_phase(self);
    }

    pub fn requires_restart(&self) -> bool {
        self.handle
            .as_ref()
            .and_then(StarRailCaptureHandle::native_failure)
            .is_some()
    }

    fn native_failure(&self) -> Option<UiError> {
        let phase = match self.phase {
            CapturePhase::Initializing => Some(UiText::new(
                "正在初始化星穹铁道抓包",
                "Initializing Star Rail capture",
            )),
            CapturePhase::Waiting => Some(UiText::new(
                "正在等待星穹铁道数据",
                "Waiting for Star Rail data",
            )),
            CapturePhase::Stopping => Some(UiText::new(
                "正在停止星穹铁道抓包",
                "Stopping Star Rail capture",
            )),
            CapturePhase::Exporting => Some(UiText::new(
                "正在导出星穹铁道数据",
                "Exporting Star Rail data",
            )),
            CapturePhase::Idle | CapturePhase::Done { .. } | CapturePhase::Failed(_) => None,
        };
        self.handle.as_ref().and_then(|handle| {
            handle.surface_native_failure(phase);
            handle.native_failure()
        })
    }

    #[cfg(feature = "test-as-invoker")]
    #[doc(hidden)]
    pub fn inject_completed_for_test(
        &mut self,
        references: ReferenceCache,
        completed_ids: Vec<u32>,
        inventory: hsr_scanner::ObservationSnapshot,
    ) {
        let achievement_count = completed_ids.len();
        *lock_shared(&self.shared) = HsrCaptureState {
            capturing: false,
            complete: true,
            achievement_count,
            completed_ids,
            inventory: Some(inventory),
            has_achievements: true,
            ..HsrCaptureState::default()
        };
        *lock_shared(&self.references) = Some(references);
        self.phase = CapturePhase::Initializing;
    }

    #[cfg(feature = "test-as-invoker")]
    #[doc(hidden)]
    pub fn completed_export_path_for_test(&self) -> Option<&str> {
        match &self.phase {
            CapturePhase::Done { path, .. } => {
                path.split("\n→ ").find(|p| p.contains("star_rail_export_"))
            },
            CapturePhase::Failed(error) => panic!("capture export failed: {error:?}"),
            _ => None,
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    lang: Lang,
    settings: &mut StarRailSettings,
    state: &mut StarRailCaptureState,
    game_busy: bool,
    restart_required: bool,
) {
    ui.add_space(4.0);
    if restart_required {
        if let Some(error) = state.native_failure() {
            widgets::error_card(ui, lang, &error);
        } else {
            ui.colored_label(
                egui::Color32::from_rgb(220, 90, 90),
                lang.t(
                    "另一个任务发生了底层崩溃。请复制完整错误，然后重启程序。",
                    "Another task had a low-level crash. Copy the full error, then restart the application.",
                ),
            );
        }
        ui.add_enabled(
            false,
            egui::Button::new(lang.t("需重启程序", "Restart required")),
        );
        return;
    }

    action_bar(ui, lang, settings, state, game_busy);
    let progress = shared_snapshot(&state.shared);
    ui.horizontal_wrapped(|ui| {
        for (label, selected, ready, count) in [
            (
                lang.t("角色", "Characters"),
                settings.capture_include_characters,
                progress.has_characters,
                progress.character_count,
            ),
            (
                lang.t("光锥", "Light Cones"),
                settings.capture_include_light_cones,
                progress.has_light_cones,
                progress.light_cone_count,
            ),
            (
                lang.t("遗器", "Relics"),
                settings.capture_include_relics,
                progress.has_relics,
                progress.relic_count,
            ),
            (
                lang.t("成就", "Achievements"),
                settings.capture_include_achievements,
                progress.has_achievements,
                progress.achievement_count,
            ),
        ] {
            if !selected {
                continue;
            }
            let value = if ready {
                count.to_string()
            } else if !progress.capturing {
                "—".to_owned()
            } else {
                lang.t("等待中", "Waiting").to_owned()
            };
            ui.label(format!("{label}: {value}"));
        }
    });
    if progress.capturing {
        ui.label(if progress.packet_count == 0 {
            lang.t("等待游戏连接…", "Waiting for the game connection…")
                .to_owned()
        } else if progress.command_count == 0 {
            lang.t(
                "已收到流量，等待登录解密…",
                "Traffic received; waiting for login decryption…",
            )
            .to_owned()
        } else {
            lang.t("已连接，正在读取游戏数据…", "Connected; reading game data…")
                .to_owned()
        });
    }
    ui.label(
        egui::RichText::new(lang.t(
            "登录游戏即可读取角色、光锥、遗器和已完成成就，完成后自动保存导出文件。",
            "Log in to capture characters, Light Cones, Relics, and completed achievements. The export is saved automatically.",
        ))
        .color(egui::Color32::from_rgb(120, 120, 120)),
    );
    ui.add_space(4.0);
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::CollapsingHeader::new(lang.t("导出设置", "Export Settings"))
                .default_open(true)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!state.is_busy() && !game_busy, |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut settings.capture_include_characters, lang.t("角色", "Characters"));
                            ui.add_space(12.0);
                            ui.checkbox(&mut settings.capture_include_light_cones, lang.t("光锥", "Light Cones"));
                            ui.add_space(12.0);
                            ui.checkbox(&mut settings.capture_include_relics, lang.t("遗器", "Relics"));
                            ui.add_space(12.0);
                            ui.checkbox(&mut settings.capture_include_achievements, lang.t("成就", "Achievements"));
                        });
                    });
                });

            egui::CollapsingHeader::new(lang.t("高级设置", "Advanced"))
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!state.is_busy() && !game_busy, |ui| {
                        ui.checkbox(&mut settings.capture_dump_packets,
                            lang.t("保存所有数据包 → debug_capture/hsr/", "Dump decrypted packets → debug_capture/hsr/"));
                        ui.checkbox(&mut settings.capture_only_keep_latest_export,
                            lang.t("仅保留最新导出", "Only keep latest export"));
                    });
                });
            ui.label(lang.t(
                "库存：Fribbels / HSR-Scanner v4；成就：StarDB。另存 GGStarRail 文件。",
                "Inventory: Fribbels / HSR-Scanner v4. Achievements: StarDB. A GGStarRail file is also saved."));

            egui::CollapsingHeader::new(lang.t("使用说明", "How to Use"))
                .default_open(true)
                .show(ui, |ui| {
                    ui.label(lang.t(
                        "1. 关闭星穹铁道。\n2. 点击“开始抓包”。\n3. 启动游戏并登录，直至进入列车或当前场景。\n4. 所选数据读取完成后会自动停止并导出。",
                        "1. Close Star Rail.\n2. Select Start Capture.\n3. Launch and log in until the Astral Express or current scene appears.\n4. Capture stops and exports after all selected data arrive.",
                    ));
                });
        });
}

fn action_bar(
    ui: &mut egui::Ui,
    lang: Lang,
    settings: &StarRailSettings,
    state: &mut StarRailCaptureState,
    game_busy: bool,
) {
    match &state.phase {
        CapturePhase::Idle => {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !game_busy
                            && capture_targets(settings).any()
                            && !settings.output_dir.trim().is_empty(),
                        egui::Button::new(lang.t("▶ 开始抓包", "▶ Start Capture")),
                    )
                    .clicked()
                {
                    start_capture(settings, state);
                }
                readiness_label(ui, lang, Readiness::NotRequested);
            });
            if game_busy {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 200, 50),
                    lang.t(
                        "另一个游戏数据任务正在运行，请等待完成。",
                        "Another game-data task is running. Wait for it to finish.",
                    ),
                );
            }
        },
        CapturePhase::Initializing | CapturePhase::Waiting => {
            ui.horizontal(|ui| {
                if ui.button(lang.t("■ 停止抓包", "■ Stop Capture")).clicked() {
                    if let Some(handle) = &state.handle {
                        handle.stop();
                    }
                    state.phase = CapturePhase::Stopping;
                }
                ui.spinner();
                readiness_label(ui, lang, Readiness::Waiting);
            });
        },
        CapturePhase::Stopping => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(lang.t("正在停止抓包...", "Stopping capture..."));
            });
        },
        CapturePhase::Exporting => {
            ui.horizontal(|ui| {
                ui.spinner();
                readiness_label(ui, lang, Readiness::Complete);
                ui.label(lang.t("正在导出...", "Exporting..."));
            });
        },
        CapturePhase::Done { summary, path } => {
            let summary = summary.clone();
            let path = path.clone();
            let cleanup_busy = state.is_busy();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !cleanup_busy,
                        egui::Button::new(lang.t("↻ 重新抓包", "↻ Capture Again")),
                    )
                    .clicked()
                {
                    state.handle = None;
                    state.phase = CapturePhase::Idle;
                }
                if cleanup_busy {
                    ui.spinner();
                }
                readiness_label(ui, lang, Readiness::Complete);
            });
            ui.colored_label(egui::Color32::from_rgb(100, 200, 100), summary.text(lang));
            ui.label(egui::RichText::new(format!("→ {path}")).small().weak());
        },
        CapturePhase::Failed(error) => {
            let error = error.clone();
            let cleanup_busy = state.is_busy();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !cleanup_busy,
                        egui::Button::new(lang.t("↻ 重试", "↻ Retry")),
                    )
                    .clicked()
                {
                    state.handle = None;
                    state.phase = CapturePhase::Idle;
                }
                if cleanup_busy {
                    ui.spinner();
                    ui.label(lang.t(
                        "正在安全关闭后台任务...",
                        "Waiting for the background task to stop safely...",
                    ));
                }
            });
            widgets::error_card(ui, lang, &error);
        },
    }
}

#[derive(Clone, Copy)]
enum Readiness {
    NotRequested,
    Waiting,
    Complete,
}

fn readiness_label(ui: &mut egui::Ui, lang: Lang, readiness: Readiness) {
    let (color, text) = match readiness {
        Readiness::NotRequested => (
            egui::Color32::from_rgb(120, 120, 120),
            lang.t("等待开始", "Ready to capture").to_owned(),
        ),
        Readiness::Waiting => (
            egui::Color32::from_rgb(255, 200, 50),
            lang.t("正在等待游戏数据", "Waiting for game data")
                .to_owned(),
        ),
        Readiness::Complete => (
            egui::Color32::from_rgb(100, 200, 100),
            lang.t("数据读取完成", "Capture complete").to_owned(),
        ),
    };
    ui.colored_label(color, text);
}

fn capture_targets(settings: &StarRailSettings) -> CaptureTargets {
    CaptureTargets {
        characters: settings.capture_include_characters,
        light_cones: settings.capture_include_light_cones,
        relics: settings.capture_include_relics,
        achievements: settings.capture_include_achievements,
    }
}

fn start_capture(settings: &StarRailSettings, state: &mut StarRailCaptureState) {
    // Freeze export destination at Start. Shared settings are disabled while
    // any Star Rail task runs, but this snapshot also protects future callers.
    state.output_dir.clone_from(&settings.output_dir);
    state.only_keep_latest_export = settings.capture_only_keep_latest_export;
    *lock_shared(&state.shared) = HsrCaptureState::default();
    *lock_shared(&state.references) = None;
    state.pending_export = None;
    state.phase = CapturePhase::Initializing;
    let native_crash = Arc::new(worker::NativeCrashState::new());
    let startup_gate = Arc::new(CaptureStartupGate::default());
    match spawn_capture_monitor(
        capture_targets(settings),
        settings.capture_dump_packets,
        state.shared.clone(),
        state.references.clone(),
        startup_gate.clone(),
        native_crash.clone(),
    ) {
        Ok((thread, command_tx)) => {
            state.handle = Some(StarRailCaptureHandle {
                thread,
                command_tx: Mutex::new(Some(command_tx)),
                startup_gate,
                native_crash,
                native_failure: Mutex::new(None),
            });
        },
        Err(error) => state.phase = CapturePhase::Failed(error),
    }
}

fn spawn_capture_monitor(
    targets: CaptureTargets,
    dump_packets: bool,
    shared: Arc<Mutex<HsrCaptureState>>,
    references_out: Arc<Mutex<Option<ReferenceCache>>>,
    startup_gate: Arc<CaptureStartupGate>,
    native_crash: Arc<worker::NativeCrashState>,
) -> Result<
    (
        std::thread::JoinHandle<()>,
        tokio::sync::mpsc::UnboundedSender<HsrCaptureCommand>,
    ),
    UiError,
> {
    let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
    let ui_command_tx = command_tx.clone();
    let shared_for_thread = shared.clone();
    let thread_crash = native_crash.clone();
    let thread = std::thread::Builder::new()
        .name("star-rail-capture".to_owned())
        .spawn(move || {
            let native_guard_active = worker::activate_native_crash(&thread_crash);
            if !native_guard_active {
                lock_shared(&shared_for_thread).error = Some(HsrError::new(
                    "HSR-ACHIEVEMENT-CAPTURE-BUSY",
                    LocalizedText::new(
                        "另一个游戏数据任务仍在关闭。请稍候重试。",
                        "Another game-data task is still shutting down. Retry shortly.",
                    ),
                    "native crash boundary is still owned by another task",
                ));
                return;
            }
            let native_context =
                native_guard_active.then(yas::native_crash::inherit_current_task);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let references = match star_rail_worker::load_references() {
                    Ok(references) => references,
                    Err(error) => {
                        lock_shared(&shared_for_thread).error = Some(error);
                        return;
                    },
                };
                let monitor = match HsrCaptureMonitor::new(
                    shared_for_thread.clone(),
                    references.clone(),
                    targets,
                ) {
                    Ok(monitor) => monitor,
                    Err(error) => {
                        lock_shared(&shared_for_thread).error = Some(error);
                        return;
                    },
                };
                let monitor = if dump_packets {
                    monitor.with_packet_dump(genshin_scanner::cli::exe_dir().join("debug_capture").join("hsr"))
                } else { monitor };
                *lock_shared(&references_out) = Some(references);

                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        lock_shared(&shared_for_thread).error = Some(HsrError::new(
                            "HSR-ACHIEVEMENT-CAPTURE-RUNTIME",
                            LocalizedText::new(
                                "抓包运行环境无法启动。请检查系统资源后重试。",
                                "The capture runtime could not start. Check system resources, then retry.",
                            ),
                            format!("tokio runtime construction failed; cause={error}"),
                        ));
                        return;
                    },
                };
                let start_sent = startup_gate.start_if_allowed(|| {
                    command_tx
                        .send(HsrCaptureCommand::StartCapture)
                        .is_ok()
                });
                drop(command_tx);
                if !start_sent {
                    return;
                }
                runtime.block_on(monitor.run(command_rx));
            }));
            if let Err(panic_info) = result {
                let details = if let Some(message) = panic_info.downcast_ref::<&str>() {
                    (*message).to_owned()
                } else if let Some(message) = panic_info.downcast_ref::<String>() {
                    message.clone()
                } else {
                    "unknown non-string panic payload".to_owned()
                };
                lock_shared(&shared_for_thread).error = Some(HsrError::new(
                    "HSR-ACHIEVEMENT-CAPTURE-PANIC",
                    LocalizedText::new(
                        "抓包任务意外停止。请重试，并在问题再次出现时复制完整错误。",
                        "The capture task stopped unexpectedly. Retry, and copy the full error if it happens again.",
                    ),
                    format!("capture monitor panicked; cause={details}"),
                ));
            }
            drop(native_context);
            if native_guard_active {
                worker::deactivate_native_crash(&thread_crash);
            }
        })
        .map_err(|error| {
            UiError::from_error(
                UiText::new(
                    "抓包后台任务无法启动。请检查系统资源后重试。",
                    "The capture background task could not start. Check system resources, then retry.",
                ),
                error,
            )
        })?;
    Ok((thread, ui_command_tx))
}

fn update_phase(state: &mut StarRailCaptureState) {
    if let Some(error) = state.native_failure() {
        state.phase = CapturePhase::Failed(error);
        return;
    }

    if state.phase == CapturePhase::Stopping {
        if state
            .handle
            .as_ref()
            .is_none_or(StarRailCaptureHandle::is_finished)
        {
            state.handle = None;
            state.phase = CapturePhase::Idle;
        }
        return;
    }

    if state.phase == CapturePhase::Exporting {
        if let Some(pending) = &mut state.pending_export {
            if pending.result.is_none() {
                match pending.receiver.try_recv() {
                    Ok(result) => pending.result = Some(result),
                    Err(mpsc::TryRecvError::Disconnected) if pending.thread.is_finished() => {
                        pending.result = Some(Err(UiError::from_message(
                            UiText::new(
                                "数据导出任务意外停止。请复制完整错误并报告问题。",
                                "The data export task stopped unexpectedly. Copy the full error and report the problem.",
                            ),
                            "data export worker disconnected without returning a result",
                        )));
                    },
                    Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => {},
                }
            }
        }

        let export_finished = state
            .pending_export
            .as_ref()
            .is_some_and(|pending| pending.result.is_some() && pending.thread.is_finished());
        if export_finished {
            let pending = state
                .pending_export
                .take()
                .expect("finished export must still be retained");
            let result = pending
                .result
                .expect("finished retained export must have a result");
            let _ = pending.thread.join();
            match result {
                Ok(files) => {
                    let summary = UiText::new("已导出所选数据。", "Selected data exported.");
                    state.phase = CapturePhase::Done {
                        summary,
                        path: files
                            .paths
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join("\n→ "),
                    };
                },
                Err(error) => state.phase = CapturePhase::Failed(error),
            }
        }
        return;
    }

    let shared = shared_snapshot(&state.shared);
    if let Some(error) = shared.error {
        state.phase = CapturePhase::Failed(star_rail_worker::hsr_ui_error(
            UiText::new(
                "星穹铁道抓包未能完成。请复制完整错误以搜索或寻求帮助。",
                "Star Rail capture could not finish. Copy the full error to search or ask for help.",
            ),
            error,
        ));
        if let Some(handle) = &state.handle {
            handle.stop();
        }
        return;
    }
    if shared.complete {
        if let Some(handle) = &state.handle {
            handle.stop();
        }
        let references = lock_shared(&state.references).clone();
        let Some(references) = references else {
            state.phase = CapturePhase::Failed(UiError::from_message(
                UiText::new(
                    "游戏数据已读取，但参考数据状态丢失，无法安全导出。请重新抓包。",
                    "Game data was read, but reference state was lost and cannot be exported safely. Capture again.",
                ),
                "complete achievement state has no retained ReferenceCache",
            ));
            return;
        };
        match spawn_export(
            shared,
            references,
            PathBuf::from(state.output_dir.trim()),
            state.only_keep_latest_export,
        ) {
            Ok(pending) => {
                state.pending_export = Some(pending);
                state.phase = CapturePhase::Exporting;
            },
            Err(error) => state.phase = CapturePhase::Failed(error),
        }
    } else if shared.capturing {
        state.phase = CapturePhase::Waiting;
    }
}

fn spawn_export(
    captured: HsrCaptureState,
    references: ReferenceCache,
    output_dir: PathBuf,
    only_latest: bool,
) -> Result<PendingExport, UiError> {
    std::fs::create_dir_all(&output_dir).map_err(|error| {
        UiError::from_error(
            UiText::new(
                "无法创建星穹铁道导出文件夹。请检查路径和文件权限。",
                "The Star Rail export folder could not be created. Check the path and file permissions.",
            ),
            error,
        )
    })?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let thread = std::thread::Builder::new()
        .name("star-rail-export".to_owned())
        .spawn(move || {
            let result = (|| {
                let inventory = captured.inventory.ok_or_else(|| UiError::from_message(
                    UiText::new("库存数据缺失，请重新抓包。", "Inventory data is missing. Capture again."),
                    "completed capture has no inventory snapshot"))?;
                let scanner = if inventory.evidence.coverage.characters != hsr_scanner::CoverageLevel::Unknown
                    || inventory.evidence.coverage.light_cones != hsr_scanner::CoverageLevel::Unknown
                    || inventory.evidence.coverage.relics != hsr_scanner::CoverageLevel::Unknown {
                    Some(hsr_scanner::scanner_export::build_scanner_export(&inventory, &references, &captured.export_details)
                        .map_err(|e| star_rail_worker::hsr_ui_error(UiText::new("无法生成通用库存导出。", "Could not build the inventory interchange export."), e))?)
                } else { None };
                let observations = ValidatedObservationSnapshot::from_packet_capture(inventory)
                    .map_err(|error| star_rail_worker::hsr_ui_error(UiText::new("库存数据校验失败。", "Inventory validation failed."), error))?;
                let export = if captured.has_achievements {
                    build_achievement_snapshot(captured.completed_ids, HSR_CAPTURE_REVISION, &references)
                        .and_then(|achievements| build_export_with_achievements(observations, achievements, &references))
                } else { build_export(observations, &references) }
                .map_err(|error| star_rail_worker::hsr_ui_error(UiText::new("无法生成星穹铁道导出文件。", "The Star Rail export could not be built."), error))?;
                super::star_rail_exports::write_capture_files(&output_dir, &export, scanner.as_ref(), only_latest)
                    .map_err(|e| star_rail_worker::hsr_ui_error(UiText::new(
                        "导出文件保存或旧文件清理失败。请查看完整错误中的文件路径。",
                        "Saving exports or removing older files failed. See the file paths in the full error."), e))

            })();
            let _ = sender.send(result);
        })
        .map_err(|error| {
            UiError::from_error(
                UiText::new(
                    "数据导出后台任务无法启动。请检查系统资源后重试。",
                    "The export background task could not start. Check system resources, then retry.",
                ),
                error,
            )
        })?;
    Ok(PendingExport {
        receiver,
        thread,
        result: None,
    })
}

fn shared_snapshot<T: Clone>(shared: &Mutex<T>) -> T {
    match shared.lock() {
        Ok(value) => value.clone(),
        Err(poisoned) => {
            shared.clear_poison();
            poisoned.into_inner().clone()
        },
    }
}

fn lock_shared<T>(shared: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match shared.lock() {
        Ok(value) => value,
        Err(poisoned) => {
            shared.clear_poison();
            poisoned.into_inner()
        },
    }
}
