#[cfg(feature = "capture")]
pub mod capture_tab;
pub mod credits;
pub mod log_bridge;
pub mod log_panel;
pub mod manager_tab;
mod privilege;
pub mod scanner_tab;
pub mod state;
pub mod update_banner;
pub mod widgets;
pub mod worker;

use eframe::egui;
use state::{AppState, Lang, UpdateState};
use worker::TaskHandle;

/// Launch the GUI application.
pub fn run_gui() {
    #[cfg(feature = "capture")]
    const PRODUCT_NAME: &str = "GOODCapture Scanner";
    #[cfg(not(feature = "capture"))]
    const PRODUCT_NAME: &str = "GOOD Scanner";

    // Register the process-wide SEH handler early. Worker threads explicitly
    // enroll in it when they start; unregistered threads are left alone.
    #[cfg(target_os = "windows")]
    worker::install_seh_handler();

    let state = AppState::new();

    // Set global language from config
    yas::lang::set_lang(state.lang.to_str());

    // Init GUI logger (replaces env_logger in GUI mode)
    let logger = log_bridge::GuiLogger::new(
        state.scanner_log_lines.clone(),
        state.manager_log_lines.clone(),
        2000,
    );
    if let Err(error) = logger.init(state.verbose) {
        let failure = state::UiError::from_message(
            state::UiText::new(
                "程序无法启动错误记录功能。请复制完整错误并报告此问题。",
                "The application could not start its error logging. Copy the full error and report this problem.",
            ),
            error.to_string(),
        );
        show_startup_error(PRODUCT_NAME, state.lang, &failure);
        return;
    }

    // Install a panic hook that writes to the log file (the default hook
    // writes to stderr, which GUI users never see).  This covers panics on
    // ALL threads — worker, update, refresh, and GUI main.
    install_panic_hook();

    // Clean up the previous update only after logging is ready, so even this
    // non-fatal startup failure retains its readable hint and inner error.
    genshin_scanner::updater::cleanup_old_exe();

    // Kick off background update check for the executable that is running.
    #[cfg(feature = "capture")]
    const UPDATE_ASSET: &str = genshin_scanner::updater::ASSET_CAPTURE;
    #[cfg(not(feature = "capture"))]
    const UPDATE_ASSET: &str = genshin_scanner::updater::ASSET_SCANNER;
    update_banner::spawn_check(UPDATE_ASSET, &state.update_state);

    let window_title = genshin_scanner::updater::window_title(PRODUCT_NAME);

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(window_title)
        .with_inner_size([720.0, 660.0])
        .with_min_inner_size([600.0, 400.0]);
    match eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon_64.png")) {
        Ok(icon) => viewport = viewport.with_icon(std::sync::Arc::new(icon)),
        Err(error) => {
            let failure = state::UiError::from_error(
                state::UiText::new(
                    "窗口图标无法加载，但程序仍可继续使用。",
                    "The window icon could not be loaded, but the application can continue.",
                ),
                error,
            );
            log::error!(target: yas::lang::LOCALIZED_LOG_TARGET, "{}", failure.copy_text(state.lang));
        },
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    if let Err(error) = eframe::run_native(
        PRODUCT_NAME,
        options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(GuiApp::new(state)))
        }),
    ) {
        let lang = if yas::lang::is_en() {
            Lang::En
        } else {
            Lang::Zh
        };
        let failure = state::UiError::from_error(
            state::UiText::new(
                "程序窗口无法启动或意外关闭。请复制下方完整错误以搜索或寻求帮助。",
                "The application window could not start or closed unexpectedly. Copy the full error below to search or ask for help.",
            ),
            error,
        );
        log::error!(target: yas::lang::LOCALIZED_LOG_TARGET, "{}", failure.copy_text(lang));
        show_startup_error(PRODUCT_NAME, lang, &failure);
    }
}

fn show_startup_error(product_name: &str, lang: Lang, failure: &state::UiError) {
    let full_error = failure.copy_text(lang);
    let mut description = full_error.clone();
    match state::persist_error_report("startup_error.txt", &full_error) {
        Ok(path) => {
            description.push_str("\n\n");
            description.push_str(lang.t(
                "完整错误也已保存到以下文件，可打开后复制：\n",
                "The full error was also saved here so it can be opened and copied:\n",
            ));
            description.push_str(&path.display().to_string());
        },
        Err(error) => {
            description.push_str("\n\n");
            description.push_str(lang.t(
                "程序还无法保存错误报告。请拍摄此窗口，或复制其中可选择的文字。完整错误详情：\n",
                "The application also could not save the error report. Take a screenshot of this window, or copy any selectable text. Full error details:\n",
            ));
            description.push_str(&format!("{error:#}"));
        },
    }

    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(format!("{} — {}", product_name, lang.t("错误", "Error")))
        .set_description(description)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Replace the default panic hook so panics on ANY thread are written
/// to the `log::error!` logger (visible in the GUI log panel + log file).
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let technical_details = state::record_panic_details(info);
        let failure = state::UiError::from_message(
            state::UiText::new(
                "程序中的任务遇到意外内部错误，当前操作可能已停止。请复制完整错误以搜索或寻求帮助。",
                "An application task encountered an unexpected internal error and may have stopped. Copy the full error to search or ask for help.",
            ),
            technical_details,
        );
        let lang = if yas::lang::is_en() {
            Lang::En
        } else {
            Lang::Zh
        };
        log::error!(target: yas::lang::LOCALIZED_LOG_TARGET, "{}", failure.copy_text(lang));
    }));
}

#[derive(PartialEq)]
enum ActiveTab {
    Scanner,
    Manager,
    #[cfg(feature = "capture")]
    Capture,
    Credits,
}

struct GuiApp {
    state: AppState,
    active_tab: ActiveTab,
    scan_handle: Option<TaskHandle>,
    server_handle: Option<TaskHandle>,
    #[cfg(feature = "capture")]
    capture_tab: capture_tab::CaptureTabState,
}

impl GuiApp {
    fn new(state: AppState) -> Self {
        #[cfg(feature = "capture")]
        let capture_tab_state =
            capture_tab::CaptureTabState::from_config(state.output_dir.clone(), &state.user_config);
        Self {
            state,
            #[cfg(feature = "capture")]
            active_tab: ActiveTab::Capture,
            #[cfg(not(feature = "capture"))]
            active_tab: ActiveTab::Scanner,
            scan_handle: None,
            server_handle: None,
            #[cfg(feature = "capture")]
            capture_tab: capture_tab_state,
        }
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Debounced auto-save: check if config changed and save after 300ms
        #[cfg(feature = "capture")]
        self.capture_tab.sync_to_config(&mut self.state.user_config);
        self.state.auto_save_tick();

        let l = self.state.lang;

        // Top bar with tabs + language toggle
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                #[cfg(feature = "capture")]
                ui.selectable_value(
                    &mut self.active_tab,
                    ActiveTab::Capture,
                    egui::RichText::new(l.t("抓包器", "Capture")).size(20.0),
                );
                ui.selectable_value(
                    &mut self.active_tab,
                    ActiveTab::Scanner,
                    egui::RichText::new(l.t("扫描器", "Scanner")).size(20.0),
                );
                ui.selectable_value(
                    &mut self.active_tab,
                    ActiveTab::Manager,
                    egui::RichText::new(l.t("管理器", "Manager")).size(20.0),
                );

                // Right-aligned: GGArtifact link, credits tab, language toggle
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = match l {
                        Lang::Zh => "EN",
                        Lang::En => "中",
                    };
                    if ui.button(egui::RichText::new(label).size(16.0)).clicked() {
                        self.state.lang = match l {
                            Lang::Zh => Lang::En,
                            Lang::En => Lang::Zh,
                        };
                        self.state.user_config.lang = self.state.lang.to_str().to_string();
                        yas::lang::set_lang(self.state.lang.to_str());
                    }
                    ui.selectable_value(
                        &mut self.active_tab,
                        ActiveTab::Credits,
                        egui::RichText::new(l.t("致谢", "Credits")).size(20.0),
                    );
                    let ggartifact_label = l.t("打开GGArtifact", "Open GGArtifact");
                    if ui
                        .button(egui::RichText::new(format!("{ggartifact_label} ↗")).size(16.0))
                        .on_hover_text("https://ggartifact.com")
                        .clicked()
                    {
                        ui.ctx()
                            .open_url(egui::OpenUrl::new_tab("https://ggartifact.com"));
                    }
                });
            });
        });

        // Update banner (between tabs and content)
        update_banner::show(ctx, self.state.lang, &self.state.update_state);

        // Bottom panel: per-tab log area.
        // Manager tab shows manager logs; everything else shows scanner logs
        // (scanner/capture tabs, credits, plus startup/update logs).
        let log_buf = match self.active_tab {
            ActiveTab::Manager => &self.state.manager_log_lines,
            _ => &self.state.scanner_log_lines,
        };
        egui::TopBottomPanel::bottom("logs")
            .min_height(120.0)
            .default_height(230.0)
            .resizable(true)
            .show(ctx, |ui| {
                log_panel::show_with(ui, self.state.lang, log_buf);
            });

        // Check cross-tab running states for mutual exclusion
        let is_scan_running = self
            .scan_handle
            .as_ref()
            .map_or(false, |h| !h.is_finished());
        let is_server_running = self
            .server_handle
            .as_ref()
            .map_or(false, |h| !h.is_finished());
        // A native worker crash exits the thread without running Rust/native
        // cleanup. Block every game-facing task until the whole application
        // is restarted; retrying in the same process is not safe.
        let restart_required = self
            .scan_handle
            .as_ref()
            .map_or(false, TaskHandle::requires_restart)
            || self
                .server_handle
                .as_ref()
                .map_or(false, TaskHandle::requires_restart);
        #[cfg(feature = "capture")]
        let restart_required = restart_required || self.capture_tab.requires_restart();
        #[cfg(feature = "capture")]
        let is_capture_busy = self.capture_tab.is_busy();
        #[cfg(not(feature = "capture"))]
        let is_capture_busy = false;

        // Central panel: active tab content
        egui::CentralPanel::default().show(ctx, |ui| match self.active_tab {
            ActiveTab::Scanner => {
                scanner_tab::show(
                    ui,
                    &mut self.state,
                    &mut self.scan_handle,
                    is_server_running || is_capture_busy,
                    restart_required,
                );
            },
            ActiveTab::Manager => {
                manager_tab::show(
                    ui,
                    &mut self.state,
                    &mut self.server_handle,
                    is_scan_running || is_capture_busy,
                    restart_required,
                );
            },
            #[cfg(feature = "capture")]
            ActiveTab::Capture => {
                capture_tab::show(
                    ui,
                    self.state.lang,
                    &mut self.capture_tab,
                    is_scan_running || is_server_running,
                    restart_required,
                );
            },
            ActiveTab::Credits => {
                #[cfg(feature = "capture")]
                credits::show(ui, l, credits::CreditSet::Full);
                #[cfg(not(feature = "capture"))]
                credits::show(ui, l, credits::CreditSet::Scanner);
            },
        });

        // Request repaint while tasks or update check are in progress
        let update_busy = matches!(
            *self.state.update_state.lock().unwrap(),
            UpdateState::Checking | UpdateState::Downloading | UpdateState::ShowingDialog,
        );
        let config_save_pending = self.state.config_dirty_since.is_some();
        let any_running = is_scan_running
            || is_server_running
            || is_capture_busy
            || update_busy
            || config_save_pending;
        if any_running {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        #[cfg(feature = "capture")]
        self.capture_tab.sync_to_config(&mut self.state.user_config);
        self.state.persist_config_now();
    }
}

/// Load system CJK font for Chinese text rendering.
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Try to load Microsoft YaHei from Windows system fonts
    let cjk_font_paths = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyh.ttf",
        "C:\\Windows\\Fonts\\simsun.ttc",
    ];

    for path in &cjk_font_paths {
        if let Ok(font_data) = std::fs::read(path) {
            fonts.font_data.insert(
                "system_cjk".to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(font_data)),
            );
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .push("system_cjk".to_owned());
            fonts
                .families
                .get_mut(&egui::FontFamily::Monospace)
                .unwrap()
                .push("system_cjk".to_owned());
            break;
        }
    }

    ctx.set_fonts(fonts);
}
