use eframe::egui;

use crate::config::{StarRailCaptureMethod, StarRailSettings};

use super::{
    star_rail_state::StarRailState,
    star_rail_worker,
    state::{Lang, TaskStatus},
    widgets, worker,
};

pub fn show(
    ui: &mut egui::Ui,
    lang: Lang,
    settings: &mut StarRailSettings,
    state: &mut StarRailState,
    game_busy: bool,
    restart_required: bool,
) {
    let is_running = state.scan_running();
    let native_failure = state
        .scan_handle
        .as_ref()
        .and_then(super::worker::TaskHandle::native_failure);

    ui.add_space(4.0);
    if restart_required {
        if let Some(error) = native_failure {
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

    ui.horizontal(|ui| {
        if is_running {
            let stopping = state
                .scan_handle
                .as_ref()
                .is_some_and(super::worker::TaskHandle::is_stopping);
            if ui
                .add_enabled(!stopping, egui::Button::new(lang.t("■ 停止", "■ Stop")))
                .clicked()
            {
                if let Some(handle) = &state.scan_handle {
                    handle.stop();
                }
            }
        } else {
            let has_target = settings.scan_characters
                || settings.scan_light_cones
                || settings.scan_relics_and_ornaments;
            if ui
                .add_enabled(
                    !game_busy && has_target,
                    egui::Button::new(lang.t("▶ 开始扫描", "▶ Start Scan")),
                )
                .clicked()
            {
                state.scan_handle = Some(star_rail_worker::spawn_scan(
                    settings,
                    state.scan_status.clone(),
                ));
            }
        }

        if let Some(status) = worker::try_task_status(&state.scan_status) {
            match status {
                TaskStatus::Running(message) => {
                    ui.spinner();
                    ui.label(message.text(lang));
                },
                TaskStatus::Completed(message) => {
                    ui.colored_label(egui::Color32::from_rgb(100, 200, 100), message.text(lang));
                },
                TaskStatus::Idle | TaskStatus::Failed(_) => {},
            }
        }
    });

    if game_busy && !is_running {
        ui.colored_label(
            egui::Color32::from_rgb(255, 200, 50),
            lang.t(
                "另一个游戏数据任务正在运行，请等待完成。",
                "Another game-data task is running. Wait for it to finish.",
            ),
        );
    }
    if let Some(TaskStatus::Failed(error)) = worker::try_task_status(&state.scan_status) {
        widgets::error_card(ui, lang, &error);
    }
    ui.label(
        egui::RichText::new(lang.t(
            "扫描角色、光锥、隧洞遗器和位面饰品，并生成可直接导入 GGStarRail 的隐私安全 JSON。",
            "Scan Characters, Light Cones, Cavern Relics, and Planar Ornaments into a privacy-safe JSON file for GGStarRail.",
        ))
        .color(egui::Color32::from_rgb(120, 120, 120)),
    );
    ui.add_enabled_ui(!is_running && !game_busy, |ui| {
        widgets::star_rail_game_data_refresh_control(ui, lang, &mut state.data_cache_refresh);
    });
    ui.add_space(4.0);
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            egui::CollapsingHeader::new(lang.t("扫描目标", "Scan Targets"))
                .default_open(true)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!is_running && !game_busy, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(
                                &mut settings.scan_characters,
                                lang.t("角色", "Characters"),
                            );
                            ui.checkbox(
                                &mut settings.scan_light_cones,
                                lang.t("光锥", "Light Cones"),
                            );
                            ui.checkbox(
                                &mut settings.scan_relics_and_ornaments,
                                lang.t(
                                    "隧洞遗器与位面饰品",
                                    "Cavern Relics & Planar Ornaments",
                                ),
                            );
                        });
                        if !settings.scan_characters
                            && !settings.scan_light_cones
                            && !settings.scan_relics_and_ornaments
                        {
                            ui.colored_label(
                                egui::Color32::from_rgb(255, 200, 50),
                                lang.t(
                                    "请至少选择一种扫描目标。",
                                    "Select at least one scan target.",
                                ),
                            );
                        }
                    });
                });

            egui::CollapsingHeader::new(lang.t("扫描设置", "Scan Settings"))
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!is_running && !game_busy, |ui| {
                        egui::Grid::new("star_rail_scan_settings")
                            .num_columns(2)
                            .spacing([12.0, 6.0])
                            .show(ui, |ui| {
                                ui.label(lang.t("截图方式", "Capture method"));
                                egui::ComboBox::from_id_salt("star_rail_capture_method")
                                    .selected_text(capture_method_label(
                                        lang,
                                        settings.capture_method,
                                    ))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut settings.capture_method,
                                            StarRailCaptureMethod::Wgc,
                                            capture_method_label(
                                                lang,
                                                StarRailCaptureMethod::Wgc,
                                            ),
                                        );
                                        ui.selectable_value(
                                            &mut settings.capture_method,
                                            StarRailCaptureMethod::BitBlt,
                                            capture_method_label(
                                                lang,
                                                StarRailCaptureMethod::BitBlt,
                                            ),
                                        );
                                        ui.selectable_value(
                                            &mut settings.capture_method,
                                            StarRailCaptureMethod::PrintWindow,
                                            capture_method_label(
                                                lang,
                                                StarRailCaptureMethod::PrintWindow,
                                            ),
                                        );
                                    });
                                ui.end_row();

                                ui.label(lang.t("操作后等待 (ms)", "Navigation delay (ms)"));
                                ui.add(
                                    egui::DragValue::new(&mut settings.navigation_delay_ms)
                                        .range(50..=5_000)
                                        .speed(10.0),
                                );
                                ui.end_row();

                                ui.label(lang.t("面板等待上限 (ms)", "Panel timeout (ms)"));
                                ui.add(
                                    egui::DragValue::new(&mut settings.panel_timeout_ms)
                                        .range(250..=10_000)
                                        .speed(10.0),
                                );
                                ui.end_row();

                                ui.label(lang.t("背包物品安全上限", "Inventory safety limit"));
                                ui.add(
                                    egui::DragValue::new(&mut settings.max_inventory_items)
                                        .range(1..=10_000),
                                );
                                ui.end_row();

                                ui.label(lang.t("角色安全上限", "Character safety limit"));
                                ui.add(
                                    egui::DragValue::new(&mut settings.max_characters)
                                        .range(1..=500),
                                );
                                ui.end_row();

                                ui.label(lang.t(
                                    "已知角色总数（0 = 未知）",
                                    "Known Character total (0 = unknown)",
                                ));
                                ui.add(
                                    egui::DragValue::new(&mut settings.expected_characters)
                                        .range(0..=500),
                                );
                                ui.end_row();

                                ui.label(lang.t("下一个角色按键", "Next Character key"));
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut settings.next_character_key,
                                    )
                                    .desired_width(60.0)
                                    .char_limit(1),
                                );
                                ui.end_row();
                            });
                        ui.label(
                            egui::RichText::new(lang.t(
                                "只有填写准确的角色总数，并验证回到首位后，角色覆盖才会标记为完整。",
                                "Character coverage is marked complete only when the correct total is supplied and wraparound to the first Character is verified.",
                            ))
                            .small()
                            .color(egui::Color32::from_rgb(120, 120, 120)),
                        );
                    });
                });

            egui::CollapsingHeader::new(lang.t("导入已有存档", "Import Existing Archive"))
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!is_running && !game_busy, |ui| {
                        path_row(
                            ui,
                            lang.t("存档 JSON", "Archive JSON"),
                            &mut settings.offline_import_path,
                            lang.t("选择文件...", "Choose file..."),
                            false,
                        );
                        ui.label(
                            egui::RichText::new(lang.t(
                                "离线导入 Reliquary / Fribbels v4 JSON；不会读取账号、会话或原始数据包。",
                                "Import an existing Reliquary / Fribbels v4 JSON offline; account, session, and raw-packet data are never retained.",
                            ))
                            .small()
                            .color(egui::Color32::from_rgb(120, 120, 120)),
                        );
                        if ui
                            .add_enabled(
                                !game_busy && !settings.offline_import_path.trim().is_empty(),
                                egui::Button::new(lang.t(
                                    "导入并导出 GGStarRail JSON",
                                    "Import and Export GGStarRail JSON",
                                )),
                            )
                            .clicked()
                        {
                            state.scan_handle = Some(star_rail_worker::spawn_offline_import(
                                settings,
                                state.scan_status.clone(),
                            ));
                        }
                    });
                });
        });
}

fn capture_method_label(lang: Lang, method: StarRailCaptureMethod) -> &'static str {
    match method {
        StarRailCaptureMethod::Wgc => lang.t(
            "Windows 图形捕获（推荐）",
            "Windows Graphics Capture (recommended)",
        ),
        StarRailCaptureMethod::BitBlt => "BitBlt",
        StarRailCaptureMethod::PrintWindow => "PrintWindow",
    }
}

pub(super) fn path_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    button_label: &str,
    directory: bool,
) {
    ui.horizontal(|ui| {
        ui.label(format!("{label}:"));
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width((ui.available_width() - 120.0).max(120.0)),
        );
        if ui.button(button_label).clicked() {
            let selected = if directory {
                rfd::FileDialog::new().pick_folder()
            } else {
                rfd::FileDialog::new()
                    .add_filter("JSON", &["json"])
                    .pick_file()
            };
            if let Some(path) = selected {
                *value = path.display().to_string();
            }
        }
    });
}
