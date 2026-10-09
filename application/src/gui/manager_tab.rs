use std::sync::atomic::Ordering;

use eframe::egui;

use super::state::{AppState, TaskStatus, UiError, UiText};
use super::widgets;
use super::worker::{self, TaskHandle};

pub fn show_settings(ui: &mut egui::Ui, state: &mut AppState, is_server_running: bool) {
    let l = state.lang;

    widgets::section(ui, l.t("连接", "Connection"), |ui| {
        widgets::field_row(ui, l.t("端口", "Port"), |ui| {
            ui.add(
                egui::DragValue::new(&mut state.server_port)
                    .range(1024..=65535)
                    .speed(0.0),
            );
        });
    });
    widgets::section(ui, l.t("执行选项", "Execution"), |ui| {
        ui.add_enabled_ui(!is_server_running, |ui| {
            ui.checkbox(&mut state.filter_involved_sets, l.t("仅筛选涉及的套装", "Filter target sets only"))
                .on_hover_text(l.t("加解锁时只扫描涉及的套装，速度更快；不会更新完整背包。", "Scan only the requested artifact sets for faster lock changes. The full inventory will not be updated."));
            ui.checkbox(&mut state.hdr_mode, l.t("游戏使用 HDR", "Game uses HDR"));
        });
    });
    widgets::section(ui, l.t("角色信息", "Character names"), |ui| {
        widgets::character_names_section(ui, state, !is_server_running);
    });

    // === Timing Delays ===
    //
    // Scan API runs the same character/weapon/artifact scanners as the
    // scanner tab, so all their delays apply when the server executes a
    // scan job. Layout: character + inventory + manager in one row.
    widgets::fold(ui, l.t("延迟设置", "Timing"), |ui| {
        ui.add_enabled_ui(!is_server_running, |ui| {
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
                            widgets::delay_group(ui, "mgr_delays", l.t("管理器", "Manager"), l, &mut [
                                (l.t("画面切换", "Screen transition"), &mut state.user_config.mgr_transition_delay, defaults.mgr_transition_delay,
                                    l.t("打开/关闭角色面板等大画面切换后的等待", "Wait after major screen transitions like opening/closing character panel")),
                                (l.t("操作按钮", "Action button"), &mut state.user_config.mgr_action_delay, defaults.mgr_action_delay,
                                    l.t("点击锁定/装备等操作按钮后的等待", "Wait after clicking action buttons like lock/equip")),
                                (l.t("格子点击", "Grid cell click"), &mut state.user_config.mgr_cell_delay, defaults.mgr_cell_delay,
                                    l.t("锁定切换后点击下一个格子前的等待", "Wait before clicking the next grid cell after a lock toggle")),
                                (l.t("滚动等待", "Scroll settle"), &mut state.user_config.mgr_scroll_delay, defaults.mgr_scroll_delay,
                                    l.t("翻页后等待物品列表稳定的时间", "Wait after scrolling for item list to stabilize")),
                            ]);
                        }
                    });
    });

    // === Advanced Options (shared with scanner tab) ===
    widgets::fold(ui, l.t("高级选项", "Advanced"), |ui| {
        ui.add_enabled_ui(!is_server_running, |ui| {
            widgets::manager_debug_options(
                ui,
                l,
                &mut state.verbose,
                &mut state.dump_images,
                &mut state.dump_job_data,
            );
        });
    });
}

/// Top action bar with port, start/stop button, and status.
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
    let status = worker::try_task_status(&state.server_status);
    if handle.as_ref().is_some_and(TaskHandle::is_stopping) {
        super::theme::task_status(
            ui,
            l,
            status.as_ref(),
            l.t("正在停止连接", "Stopping connection"),
        );
    } else {
        super::manager_progress::show(ui, l, &state.server_job, status.as_ref());
    }
    ui.add_space(16.0);
    if running {
        let stopping = handle.as_ref().is_some_and(TaskHandle::is_stopping);
        if super::theme::primary_action(
            ui,
            !stopping,
            if stopping {
                l.t("正在停止", "Stopping")
            } else {
                l.t("停止连接", "Stop connection")
            },
        )
        .clicked()
        {
            if let Some(h) = handle {
                h.stop();
            }
        }
    } else if super::theme::primary_action(ui, !game_busy, l.t("启动连接", "Start connection"))
        .clicked()
    {
        if let Err(e) = super::privilege::ensure_admin_for_action() {
            *state.server_status.lock().unwrap() = TaskStatus::Failed(UiError::from_anyhow(
                UiText::new("请以管理员身份启动程序", "Restart the app as administrator"),
                &e,
            ));
        } else {
            state.server_enabled.store(true, Ordering::Relaxed);
            state.persist_config_now();
            *handle = Some(worker::spawn_server(state));
        }
    }
}
