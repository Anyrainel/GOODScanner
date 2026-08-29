use eframe::egui;

use super::state::{AppState, TaskStatus, UiError, UiText};
use super::widgets;
use super::worker::{self, TaskHandle};

pub fn show(
    ui: &mut egui::Ui,
    state: &mut AppState,
    scan_handle: &mut Option<TaskHandle>,
    game_busy: bool,
    restart_required: bool,
) {
    let is_scanning = scan_handle.as_ref().map_or(false, |h| !h.is_finished());
    let native_failure = scan_handle.as_ref().and_then(TaskHandle::native_failure);
    let l = state.lang;

    // === Action bar (always visible at top) ===
    ui.add_space(4.0);
    action_bar(
        ui,
        state,
        scan_handle,
        is_scanning,
        game_busy,
        restart_required,
        native_failure.as_ref(),
    );
    if restart_required {
        return;
    }
    ui.colored_label(
        egui::Color32::from_rgb(120, 120, 120),
        if is_scanning {
            l.t(
                "扫描过程中可按鼠标右键终止。",
                "Right-click to abort during scanning.",
            )
        } else {
            l.t(
                "请确认游戏已运行，扫描过程中可按鼠标右键终止。",
                "Make sure the game is running. Right-click to abort during scanning.",
            )
        },
    );
    ui.add_space(4.0);
    ui.separator();

    // === Scrollable config area ===
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
        ui.add_space(4.0);

        // === Character Names (always visible, shared with manager tab) ===
        widgets::character_names_section(ui, state, !is_scanning);

        ui.add_space(8.0);

        // === Scan Targets (collapsible, horizontal) ===
        egui::CollapsingHeader::new(l.t("扫描目标", "Scan Targets"))
            .default_open(true)
            .show(ui, |ui| {
                ui.add_enabled_ui(!is_scanning, |ui| {
                    ui.label(l.t("请先清除背包内的过滤选项。", "Please clear up filters in inventory first."));
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut state.scan_characters, l.t("角色", "Characters"));
                        ui.add_space(12.0);
                        ui.checkbox(&mut state.scan_weapons, l.t("武器", "Weapons"));
                        ui.add_space(12.0);
                        ui.checkbox(&mut state.scan_artifacts, l.t("圣遗物", "Artifacts"));
                    });
                    ui.checkbox(&mut state.hdr_mode, l.t("我的原神在使用HDR", "HDR mode"));
                });
            });

        // === Timing Delays ===
        egui::CollapsingHeader::new(l.t("延迟设置", "Timing Delays"))
            .default_open(false)
            .show(ui, |ui| {
                ui.add_enabled_ui(!is_scanning, |ui| {
                    // Two delay groups side by side: Character and Inventory
                    let defaults = genshin_scanner::cli::GoodUserConfig::default();
                    ui.columns(2, |cols| {
                        widgets::delay_group(&mut cols[0], "char_delays", l.t("角色", "Character"), l, &mut [
                            (l.t("打开界面", "Open screen"), &mut state.user_config.char_open_delay, defaults.char_open_delay,
                                l.t("打开角色界面后等待完全加载的时间", "Wait time for character screen to fully load after opening")),
                            (l.t("关闭界面", "Close screen"), &mut state.user_config.char_close_delay, defaults.char_close_delay,
                                l.t("关闭角色界面后等待返回主界面的时间", "Wait time after closing character screen to return to main view")),
                            (l.t("面板切换", "Panel switch"), &mut state.user_config.char_tab_delay, defaults.char_tab_delay,
                                l.t("切换角色详情标签页（天赋/命座等）后的等待", "Wait after switching character detail tabs (talents/constellations etc.)")),
                            (l.t("切换角色", "Next character"), &mut state.user_config.char_next_delay, defaults.char_next_delay,
                                l.t("切换到下一个角色后等待面板更新的时间", "Wait after switching to next character for panel to update")),
                        ]);
                        widgets::inventory_delays(&mut cols[1], state, l);
                    });
                });
            });

        // === Advanced Options ===
        egui::CollapsingHeader::new(l.t("高级选项", "Advanced Options"))
            .default_open(false)
            .show(ui, |ui| {
                ui.add_enabled_ui(!is_scanning, |ui| {
                    // Checkboxes in a flowing horizontal layout
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut state.verbose, l.t("详细信息", "Verbose"));
                        ui.checkbox(&mut state.continue_on_failure, l.t("失败继续", "Continue on failure"));
                        ui.checkbox(&mut state.dump_images, l.t("保存OCR截图 → debug_images/", "Dump OCR images → debug_images/"));
                        ui.checkbox(&mut state.save_on_cancel, l.t("手动终止后依然保存文件", "Save partial results on cancel"));
                    });

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(l.t("最大扫描数 (0=全部):", "Max count (0=all):"));
                        ui.add_space(8.0);
                        ui.label(l.t("角色:", "Char:"));
                        max_count_field(ui, &mut state.char_max_count);
                        ui.add_space(8.0);
                        ui.label(l.t("武器:", "Wpn:"));
                        max_count_field(ui, &mut state.weapon_max_count);
                        ui.add_space(8.0);
                        ui.label(l.t("圣遗物:", "Art:"));
                        max_count_field(ui, &mut state.artifact_max_count);
                    });

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(l.t(
                            "OCR池数量 (0=按内存自动):",
                            "OCR pool size (0=auto by RAM):",
                        ));
                        ui.add_space(8.0);
                        ui.label("v5:");
                        pool_size_field(ui, &mut state.user_config.ocr_pool_v5_override);
                        ui.add_space(8.0);
                        ui.label("v4:");
                        pool_size_field(ui, &mut state.user_config.ocr_pool_v4_override);
                        ui.add_space(8.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(160, 160, 160),
                            l.t("下次扫描生效", "Applied on next scan"),
                        );
                    });

                    ui.add_space(4.0);
                    widgets::game_data_refresh_control(
                        ui,
                        l,
                        &mut state.mappings_refresh,
                        UiText::new(
                            "无法刷新扫描器使用的游戏数据。请检查网络连接，然后重试。",
                            "The scanner's game data could not be refreshed. Check the network connection, then retry.",
                        ),
                        genshin_scanner::scanner::common::mappings::force_refresh,
                    );
                });
            });
    });
}

/// Top action bar
fn action_bar(
    ui: &mut egui::Ui,
    state: &mut AppState,
    scan_handle: &mut Option<TaskHandle>,
    is_scanning: bool,
    game_busy: bool,
    restart_required: bool,
    native_failure: Option<&UiError>,
) {
    let l = state.lang;

    if restart_required {
        if let Some(error) = native_failure {
            widgets::error_card(ui, l, error);
        } else {
            ui.colored_label(
                egui::Color32::from_rgb(220, 90, 90),
                l.t(
                    "另一个任务发生了底层崩溃。请先复制该任务中的完整错误，然后重启本程序。",
                    "Another task had a low-level crash. Copy its full error, then restart this application.",
                ),
            );
        }
        ui.add_enabled(
            false,
            egui::Button::new(l.t("需重启程序", "Restart required")),
        );
        return;
    }

    if game_busy && !is_scanning {
        ui.colored_label(
            egui::Color32::from_rgb(255, 200, 50),
            l.t(
                "另一个游戏数据任务正在运行，请先停止后再扫描",
                "Another game-data task is running. Stop it before scanning.",
            ),
        );
    }

    ui.horizontal(|ui| {
        if is_scanning {
            let is_stopping = scan_handle.as_ref().map_or(false, |h| h.is_stopping());
            let label = if is_stopping {
                l.t("⏳ 正在停止...", "⏳ Stopping...")
            } else {
                l.t("⏹ 停止扫描", "⏹ Stop Scan")
            };
            let clicked = ui
                .add_enabled(!is_stopping, egui::Button::new(label))
                .clicked();
            if clicked {
                if let Some(ref handle) = scan_handle {
                    handle.stop();
                }
            }
            if let Some(TaskStatus::Running(phase)) =
                worker::try_task_status(&state.scan_status)
            {
                ui.spinner();
                ui.label(phase.text(l));
            }
        } else {
            let any_selected = state.scan_characters || state.scan_weapons || state.scan_artifacts;
            let can_scan = any_selected && !game_busy;
            if ui
                .add_enabled(
                    can_scan,
                    egui::Button::new(l.t("▶ 开始扫描", "▶ Start Scan")),
                )
                .clicked()
            {
                if state.missing_required_character_names() {
                    state.names_need_attention = true;
                    yas::log_warn!("旅行者为必填项", "Traveler name is required");
                } else if let Err(e) = super::privilege::ensure_admin_for_action() {
                    *state.scan_status.lock().unwrap() = TaskStatus::Failed(
                        UiError::from_anyhow(
                            UiText::new(
                                "扫描器需要管理员权限才能控制游戏。请以管理员身份重新启动程序。",
                                "The scanner needs administrator access to control the game. Restart the application as administrator.",
                            ),
                            &e,
                        ),
                    );
                } else {
                    state.names_need_attention = false;
                    // Force immediate save before scanning (don't wait for debounce)
                    state.persist_config_now();
                    *scan_handle = Some(worker::spawn_scan(state));
                }
            }
        }
    });

    match worker::try_task_status(&state.scan_status) {
        Some(TaskStatus::Completed(ref msg)) => {
            ui.colored_label(egui::Color32::from_rgb(100, 200, 100), msg.text(l));
        },
        Some(TaskStatus::Failed(ref error)) => {
            widgets::error_card(ui, l, error);
        },
        _ => {},
    }
}

fn max_count_field(ui: &mut egui::Ui, value: &mut usize) {
    ui.add(egui::DragValue::new(value).range(0..=2000).speed(0.0));
}

fn pool_size_field(ui: &mut egui::Ui, value: &mut usize) {
    ui.add(egui::DragValue::new(value).range(0..=8).speed(0.0));
}
