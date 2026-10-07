use eframe::egui;
use hsr_scanner::TrailblazerGender;

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
                    !game_busy && has_target && !settings.missing_trailblazer(),
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
            "扫描角色、光锥、隧洞遗器和位面饰品，并生成与抓包相同的 HSR-Scanner v4 JSON（star_rail_scan_*.json）。",
            "Scan Characters, Light Cones, Cavern Relics, and Planar Ornaments into the same HSR-Scanner v4 JSON as capture (star_rail_scan_*.json).",
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
                        if settings.scan_characters {
                            trailblazer_row(ui, lang, settings);
                            ui.label(lang.t(
                                "扫描会自动点击并拖动顶部角色栏；扫描期间请勿操作鼠标或键盘。",
                                "Scanning clicks and drags the top Character bar. Keep the mouse and keyboard idle during scanning.",
                            ));
                        }
                        if ui.checkbox(
                            &mut settings.hdr_mode,
                            lang.t("我的星穹铁道在使用HDR", "HDR mode"),
                        ).on_hover_text(lang.t(
                            "切换此选项会恢复自动截图：普通模式使用 BitBlt，HDR 模式使用 Windows 图形捕获。",
                            "Changing this option restores Automatic capture: BitBlt normally, Windows Graphics Capture for HDR.",
                        )).changed() {
                            settings.set_hdr_mode(settings.hdr_mode);
                        }
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

            egui::CollapsingHeader::new(lang.t("延迟设置", "Timing Delays"))
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!is_running && !game_busy, |ui| {
                        let defaults = hsr_scanner::scan_timing::ScanTimings::default();
                        let timings = &mut settings.timings;
                        ui.columns(2, |cols| {
                            widgets::delay_group(&mut cols[0], "hsr_timing_0", lang.t("界面操作", "Navigation"), lang, &mut [
                                (lang.t("打开界面", "Open screen"), &mut timings.menu_open_ms, defaults.menu_open_ms, lang.t("打开背包或角色界面后的总等待时间", "Total wait after opening Inventory or Characters")),
                                (lang.t("关闭界面", "Close screen"), &mut timings.menu_close_ms, defaults.menu_close_ms, lang.t("关闭界面或弹窗后的等待时间", "Wait after closing a menu or dialog")),
                                (lang.t("输入就绪", "Input ready"), &mut timings.input_settle_ms, defaults.input_settle_ms, lang.t("切换鼠标操作模式或激活窗口后等待输入生效", "Wait after activating the window or switching input mode")),
                                (lang.t("背包分类切换", "Inventory tab"), &mut timings.inventory_tab_ms, defaults.inventory_tab_ms, lang.t("切换光锥或遗器分类后的等待时间", "Wait after switching Light Cone or Relic tabs")),
                                (lang.t("详情面板切换", "Panel switch"), &mut timings.panel_switch_ms, defaults.panel_switch_ms, lang.t("角色详情、星魂以及首次选中背包物品共用此等待", "Shared wait for Character details, Eidolons and the first inventory selection")),
                                (lang.t("打开行迹", "Open Traces"), &mut timings.traces_open_ms, defaults.traces_open_ms, lang.t("行迹界面加载后的等待时间", "Wait for the Traces screen to load")),
                                (lang.t("角色栏翻页", "Character page"), &mut timings.character_page_ms, defaults.character_page_ms, lang.t("角色栏拖动后的等待，扫描和拖动诊断共用", "Wait after dragging the Character bar, shared with the drag diagnostic")),
                            ]);
                            widgets::delay_group(&mut cols[1], "hsr_timing_1", lang.t("截图与验证", "Capture & Verification"), lang, &mut [
                                (lang.t("截图间隔", "Capture interval"), &mut timings.capture_interval_ms, defaults.capture_interval_ms, lang.t("比较稳定画面或选中框的两次截图之间的等待", "Wait between screenshots used to confirm a stable panel or selection")),
                                (lang.t("按键后等待", "Key settle"), &mut timings.key_settle_ms, defaults.key_settle_ms, lang.t("发送下一项按键后，开始检查画面前的等待", "Wait after next-item input before checking the screen")),
                                (lang.t("面板检查间隔", "Panel poll interval"), &mut timings.poll_interval_ms, defaults.poll_interval_ms, lang.t("等待物品或角色切换时检查画面的间隔", "Interval for checking an item or Character transition")),
                                (lang.t("选中框动画", "Selection animation"), &mut timings.selection_settle_ms, defaults.selection_settle_ms, lang.t("相同物品副本之间切换时，选中框动画的等待时间", "Selection animation wait when moving between identical item copies")),
                                (lang.t("面板等待上限", "Panel timeout"), &mut timings.panel_timeout_ms, defaults.panel_timeout_ms, lang.t("单次切换等待画面更新的最长时间", "Maximum wait for one item or Character transition")),
                                (lang.t("界面检查间隔", "Menu poll interval"), &mut timings.menu_poll_interval_ms, defaults.menu_poll_interval_ms, lang.t("界面标题尚未加载时重新检查的间隔", "Interval for rechecking a menu title while it loads")),
                                (lang.t("标记切换", "Status toggle"), &mut timings.status_toggle_ms, defaults.status_toggle_ms, lang.t("管理器切换锁定或弃置标记后共用的等待", "Shared manager wait after toggling Lock or Discard")),
                            ]);
                        });
                    });
                });

            egui::CollapsingHeader::new(lang.t("高级选项", "Advanced Options"))
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!is_running && !game_busy, |ui| {
                        egui::Grid::new("star_rail_scan_settings")
                            .num_columns(2)
                            .spacing([12.0, 6.0])
                            .show(ui, |ui| {
                                ui.label(lang.t("截图方式覆盖", "Capture override"));
                                egui::ComboBox::from_id_salt("star_rail_capture_method")
                                    .selected_text(capture_method_label(
                                        lang,
                                        settings.capture_method,
                                    ))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut settings.capture_method,
                                            StarRailCaptureMethod::Auto,
                                            capture_method_label(lang, StarRailCaptureMethod::Auto),
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
                                            StarRailCaptureMethod::Wgc,
                                            capture_method_label(
                                                lang,
                                                StarRailCaptureMethod::Wgc,
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

                                for (zh, en, value) in [
                                    ("角色", "Characters", &mut settings.max_characters),
                                    ("光锥", "Light Cones", &mut settings.max_light_cones),
                                    ("遗器与饰品", "Relics & Ornaments", &mut settings.max_gear),
                                ] {
                                    ui.label(lang.t(zh, en));
                                    ui.add(egui::DragValue::new(value).range(0..=10_000));
                                    ui.end_row();
                                }

                            });
                        if settings.capture_method != StarRailCaptureMethod::Auto {
                            ui.label(lang.t(
                                "手动截图方式会覆盖自动选择。HDR 色调映射仅适用于 Windows 图形捕获；选择“自动”可恢复推荐设置。",
                                "A capture override replaces automatic selection. HDR tone mapping applies to WGC only; choose Automatic to restore the recommended setting.",
                            ));
                        }
                        ui.label(lang.t(
                            "最大扫描数：0 = 全部。达到上限时仅导出已扫描条目。",
                            "Max scan count: 0 = all. Reaching a cap saves partial results.",
                        ));
                        ui.checkbox(
                            &mut settings.dump_images,
                            lang.t("保存OCR截图 → debug_images/", "Dump OCR images → debug_images/"),
                        );
                    });
                });
        });
}

/// The Trailblazer can be renamed in-game, like Genshin's Traveler, and the
/// character screen does not show which gender this account plays.
fn trailblazer_row(ui: &mut egui::Ui, lang: Lang, settings: &mut StarRailSettings) {
    let missing = settings.missing_trailblazer();
    let label_color = if missing {
        egui::Color32::from_rgb(255, 100, 100)
    } else {
        ui.visuals().text_color()
    };
    ui.horizontal(|ui| {
        ui.colored_label(label_color, lang.t("开拓者昵称*", "Trailblazer nickname*"));
        ui.add(egui::TextEdit::singleline(&mut settings.trailblazer_name).desired_width(160.0));
        ui.add_space(12.0);
        ui.colored_label(label_color, lang.t("性别*", "Gender*"));
        egui::ComboBox::from_id_salt("star_rail_trailblazer_gender")
            .selected_text(match settings.trailblazer_gender {
                Some(gender) => trailblazer_gender_label(lang, gender),
                None => lang.t("请选择", "Choose"),
            })
            .show_ui(ui, |ui| {
                for gender in [TrailblazerGender::Stelle, TrailblazerGender::Caelus] {
                    ui.selectable_value(
                        &mut settings.trailblazer_gender,
                        Some(gender),
                        trailblazer_gender_label(lang, gender),
                    );
                }
            });
    });
    if missing {
        ui.colored_label(
            egui::Color32::from_rgb(255, 200, 50),
            lang.t(
                "扫描角色需要开拓者的游戏内昵称和性别。",
                "Scanning Characters needs the Trailblazer's in-game nickname and gender.",
            ),
        );
    }
}

fn trailblazer_gender_label(lang: Lang, gender: TrailblazerGender) -> &'static str {
    match gender {
        TrailblazerGender::Stelle => lang.t("星（女）", "Stelle (female)"),
        TrailblazerGender::Caelus => lang.t("穹（男）", "Caelus (male)"),
    }
}

fn capture_method_label(lang: Lang, method: StarRailCaptureMethod) -> &'static str {
    match method {
        StarRailCaptureMethod::Auto => lang.t("自动（推荐）", "Automatic (recommended)"),
        StarRailCaptureMethod::Wgc => lang.t("Windows 图形捕获", "Windows Graphics Capture"),
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
