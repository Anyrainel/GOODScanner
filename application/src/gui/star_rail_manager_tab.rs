use super::{
    star_rail_state::StarRailState,
    star_rail_worker,
    state::{Lang, TaskStatus, UiError, UiText},
    widgets, worker,
};
use crate::config::StarRailSettings;
use eframe::egui;

pub fn show_status(
    ui: &mut egui::Ui,
    lang: Lang,
    settings: &mut StarRailSettings,
    state: &mut StarRailState,
    game_busy: bool,
    restart_required: bool,
) {
    if restart_required {
        super::theme::restart_required(
            ui,
            lang,
            state
                .manager_handle
                .as_ref()
                .and_then(worker::TaskHandle::native_failure)
                .as_ref(),
        );
        return;
    }
    let running = state.manager_running();
    let status = worker::try_task_status(&state.manager_status);
    super::theme::task_status(
        ui,
        lang,
        status.as_ref(),
        lang.t("连接尚未启动", "Connection is off"),
    );
    let job = state.manager_job.lock().unwrap().clone();
    if job.kind.is_some() {
        widgets::hint(
            ui,
            &format!("{} · {}", lang.t("请求数量", "Requested"), job.count),
        );
        if let Ok(progress) = state.manager_progress.try_lock() {
            super::task_progress::show(ui, lang, &progress);
        }
    }
    if running && job.phase == "idle" {
        widgets::hint(
            ui,
            lang.t(
                "在 GGArtifact 中连接管理器并选择操作。",
                "Connect the manager in GGArtifact and choose an action.",
            ),
        );
    }
    ui.add_space(16.0);
    if running {
        let stopping = state
            .manager_handle
            .as_ref()
            .is_some_and(worker::TaskHandle::is_stopping);
        if super::theme::primary_action(
            ui,
            !stopping,
            if stopping {
                lang.t("正在停止", "Stopping")
            } else {
                lang.t("停止连接", "Stop connection")
            },
        )
        .clicked()
        {
            if let Some(handle) = &state.manager_handle {
                handle.stop();
            }
        }
    } else if super::theme::primary_action(ui, !game_busy, lang.t("启动连接", "Start connection"))
        .clicked()
    {
        if let Err(error) = super::privilege::ensure_admin_for_action() {
            *state.manager_status.lock().unwrap() = TaskStatus::Failed(UiError::from_anyhow(
                UiText::new("请以管理员身份启动程序", "Restart the app as administrator"),
                &error,
            ));
        } else {
            state.manager_handle = Some(star_rail_worker::spawn_manager_server(
                settings,
                state.manager_status.clone(),
                state.manager_progress.clone(),
                state.manager_job.clone(),
            ));
        }
    }
}

pub fn show_settings(
    ui: &mut egui::Ui,
    lang: Lang,
    settings: &mut StarRailSettings,
    _state: &mut StarRailState,
    is_running: bool,
) {
    ui.add_enabled_ui(!is_running, |ui| {
        widgets::section(ui, lang.t("连接", "Connection"), |ui| {
            widgets::field_row(ui, lang.t("端口", "Port"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut settings.manager_port)
                        .range(1024..=65535)
                        .speed(0.0),
                );
            });
        });
        widgets::section(ui, lang.t("显示设置", "Display"), |ui| {
            if ui
                .checkbox(
                    &mut settings.hdr_mode,
                    lang.t("游戏使用 HDR", "Game uses HDR"),
                )
                .changed()
            {
                settings.set_hdr_mode(settings.hdr_mode);
            }
        });
        widgets::fold(ui, lang.t("角色扫描", "Character scans"), |ui| {
            super::star_rail_scanner_tab::trailblazer_row(ui, lang, settings, false);
        });
        super::star_rail_scanner_tab::timing_settings(ui, lang, settings, false);
        widgets::fold(ui, lang.t("高级设置", "Advanced"), |ui| {
            widgets::manager_debug_options(
                ui,
                lang,
                &mut settings.verbose,
                &mut settings.dump_images,
                &mut settings.dump_job_data,
            );
        });
    });
}
