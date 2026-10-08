use eframe::egui;

use super::state::{AppState, TaskStatus, UiError, UiText};
use super::widgets;
use super::worker::{self, TaskHandle};

pub fn show_settings(ui: &mut egui::Ui, state: &mut AppState, is_scanning: bool) {
    let l = state.lang;

    widgets::section(ui, l.t("扫描内容", "Scan targets"), |ui| {
        ui.add_enabled_ui(!is_scanning, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut state.scan_characters, l.t("角色", "Characters"));
                ui.checkbox(&mut state.scan_weapons, l.t("武器", "Weapons"));
                ui.checkbox(&mut state.scan_artifacts, l.t("圣遗物", "Artifacts"));
                ui.checkbox(&mut state.scan_achievements, l.t("成就", "Achievements"));
            });
            widgets::hint(
                ui,
                l.t(
                    "扫描前请清除背包过滤条件。",
                    "Clear inventory filters first.",
                ),
            );
        });
    });
    if state.scan_characters || state.names_need_attention {
        widgets::section(ui, l.t("角色信息", "Character names"), |ui| {
            widgets::character_names_section(ui, state, !is_scanning);
        });
    }
    widgets::section(ui, l.t("导出与显示", "Export & display"), |ui| {
        ui.add_enabled_ui(!is_scanning, |ui| {
            widgets::scan_export_options(
                ui,
                l,
                &mut state.only_keep_latest_export,
                &mut state.save_on_cancel,
            );
            ui.checkbox(&mut state.hdr_mode, l.t("游戏使用 HDR", "Game uses HDR"));
        });
    });

    // === Timing Delays ===
    widgets::fold(ui, l.t("延迟设置", "Timing"), |ui| {
        ui.add_enabled_ui(!is_scanning, |ui| {
                    let defaults = genshin_scanner::cli::GoodUserConfig::default();
                    {
                        widgets::delay_group(ui, "char_delays", l.t("角色", "Character"), l, &mut [
                            (l.t("打开界面", "Open screen"), &mut state.user_config.char_open_delay, defaults.char_open_delay,
                                l.t("打开角色界面后等待完全加载的时间", "Wait time for character screen to fully load after opening")),
                            (l.t("关闭界面", "Close screen"), &mut state.user_config.char_close_delay, defaults.char_close_delay,
                                l.t("关闭角色界面后等待返回主界面的时间", "Wait time after closing character screen to return to main view")),
                            (l.t("面板切换", "Panel switch"), &mut state.user_config.char_tab_delay, defaults.char_tab_delay,
                                l.t("切换角色详情标签页（天赋/命座等）后的等待", "Wait after switching character detail tabs (talents/constellations etc.)")),
                            (l.t("切换角色", "Next character"), &mut state.user_config.char_next_delay, defaults.char_next_delay,
                                l.t("切换到下一个角色后等待面板更新的时间", "Wait after switching to next character for panel to update")),
                        ]);
                        widgets::inventory_delays(ui, state, l);
                    }
                    widgets::delay_group(ui, "achievement_delays", l.t("成就", "Achievements"), l, &mut [
                        (l.t("打开界面", "Open screen"), &mut state.user_config.achievement_open_delay, defaults.achievement_open_delay,
                            l.t("从暂停菜单打开成就界面后的等待", "Wait after opening the achievement screen from the pause menu")),
                        (l.t("列表滚动", "List scroll"), &mut state.user_config.achievement_scroll_delay, defaults.achievement_scroll_delay,
                            l.t("成就列表每次滚轮后的等待，与背包翻页无关", "Wait after each achievement-list wheel tick, independent of backpack scrolling")),
                        (l.t("分类切换", "Category switch"), &mut state.user_config.achievement_category_delay, defaults.achievement_category_delay,
                            l.t("点击左侧成就分类后的等待", "Wait after clicking a left-side achievement category")),
                    ]);
                });
    });

    // === Advanced Options ===
    widgets::fold(ui, l.t("高级选项", "Advanced"), |ui| {
        ui.add_enabled_ui(!is_scanning, |ui| {
            ui.checkbox(&mut state.verbose, l.t("详细日志", "Detailed logs"));
            ui.checkbox(
                &mut state.continue_on_failure,
                l.t("识别失败时继续", "Continue on OCR errors"),
            );
            ui.checkbox(
                &mut state.dump_images,
                l.t("保存 OCR 截图", "Save OCR screenshots"),
            );
            ui.separator();
            ui.strong(l.t("扫描上限", "Scan limits"));
            widgets::hint(ui, l.t("0 = 全部", "0 = all"));
            for (label, value) in [
                (l.t("角色", "Characters"), &mut state.char_max_count),
                (l.t("武器", "Weapons"), &mut state.weapon_max_count),
                (l.t("圣遗物", "Artifacts"), &mut state.artifact_max_count),
                (
                    l.t("成就", "Achievements"),
                    &mut state.achievement_max_count,
                ),
            ] {
                widgets::field_row(ui, label, |ui| {
                    max_count_field(ui, value);
                });
            }
            ui.separator();
            ui.strong(l.t("OCR 并发", "OCR workers"));
            widgets::hint(
                ui,
                l.t(
                    "0 = 按内存自动分配，下次扫描生效",
                    "0 = automatic. Applies on next scan.",
                ),
            );
            widgets::field_row(ui, "v5", |ui| {
                pool_size_field(ui, &mut state.user_config.ocr_pool_v5_override);
            });
            widgets::field_row(ui, "v4", |ui| {
                pool_size_field(ui, &mut state.user_config.ocr_pool_v4_override);
            });
        });
    });
}

pub fn show_status(
    ui: &mut egui::Ui,
    state: &mut AppState,
    handle: &mut Option<TaskHandle>,
    game_busy: bool,
    restart: bool,
) {
    let l = state.lang;
    if restart {
        super::theme::restart_required(
            ui,
            l,
            handle
                .as_ref()
                .and_then(TaskHandle::native_failure)
                .as_ref(),
        );
        return;
    }
    let running = handle.as_ref().is_some_and(|h| !h.is_finished());
    let mut status = worker::try_task_status(&state.scan_status);
    if matches!(status, Some(TaskStatus::AwaitingInput(_)))
        && !state.missing_required_character_names()
    {
        *state.scan_status.lock().unwrap() = TaskStatus::Idle;
        status = Some(TaskStatus::Idle);
        state.names_need_attention = false;
    }
    if matches!(status, Some(TaskStatus::Idle))
        && !state.scan_characters
        && !state.scan_weapons
        && !state.scan_artifacts
        && !state.scan_achievements
    {
        status = Some(TaskStatus::AwaitingInput(UiText::new(
            "请选择扫描内容",
            "Select scan targets",
        )));
    }
    super::theme::task_status(ui, l, status.as_ref(), l.t("准备扫描", "Ready to scan"));
    if let Ok(progress) = state.scan_progress.try_lock() {
        let mut display = progress.clone();
        if matches!(status, Some(TaskStatus::Failed(_))) {
            display.interrupt_unfinished();
        }
        super::task_progress::show(ui, l, &display);
    }
    ui.add_space(12.0);
    if running {
        let stopping = handle.as_ref().is_some_and(TaskHandle::is_stopping);
        if super::theme::primary_action(
            ui,
            !stopping,
            if stopping {
                l.t("正在停止", "Stopping")
            } else {
                l.t("停止扫描", "Stop scan")
            },
        )
        .clicked()
        {
            if let Some(h) = handle {
                h.stop();
            }
        }
    } else if super::theme::primary_action(
        ui,
        !game_busy
            && (state.scan_characters
                || state.scan_weapons
                || state.scan_artifacts
                || state.scan_achievements),
        if matches!(status, Some(TaskStatus::Failed(_))) {
            l.t("重试", "Retry")
        } else {
            l.t("开始扫描", "Start scan")
        },
    )
    .clicked()
    {
        if state.missing_required_character_names() {
            state.names_need_attention = true;
            *state.scan_status.lock().unwrap() = TaskStatus::AwaitingInput(UiText::new(
                "请填写旅行者名字",
                "Enter the Traveler's name",
            ));
            *state.scan_progress.lock().unwrap() = Default::default();
        } else if let Err(e) = super::privilege::ensure_admin_for_action() {
            *state.scan_status.lock().unwrap() = TaskStatus::Failed(UiError::from_anyhow(
                UiText::new("请以管理员身份启动程序", "Restart the app as administrator"),
                &e,
            ));
        } else {
            state.names_need_attention = false;
            state.persist_config_now();
            *handle = Some(worker::spawn_scan(state));
        }
    }
}

fn max_count_field(ui: &mut egui::Ui, value: &mut usize) {
    ui.add(egui::DragValue::new(value).range(0..=2000).speed(0.0));
}

fn pool_size_field(ui: &mut egui::Ui, value: &mut usize) {
    ui.add(egui::DragValue::new(value).range(0..=8).speed(0.0));
}
