use std::collections::BTreeSet;

use eframe::egui;
use hsr_scanner::{localization::Language, manager::MutationScope};

use crate::config::StarRailSettings;

use super::{
    star_rail_scanner_tab::path_row,
    star_rail_state::{ManagerPreview, StarRailState},
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
    invalidate_changed_preview(settings, state);
    let is_running = state.manager_running();
    let native_failure = state
        .manager_handle
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
                .manager_handle
                .as_ref()
                .is_some_and(super::worker::TaskHandle::is_stopping);
            if ui
                .add_enabled(!stopping, egui::Button::new(lang.t("■ 停止", "■ Stop")))
                .clicked()
            {
                if let Some(handle) = &state.manager_handle {
                    handle.stop();
                }
            }
        } else if ui
            .add_enabled(
                !game_busy
                    && !settings.manager_instructions_path.trim().is_empty()
                    && !settings.manager_journal_path.trim().is_empty(),
                egui::Button::new(lang.t("▶ 完整重扫并生成预览", "▶ Full Rescan and Preview")),
            )
            .clicked()
        {
            state.invalidate_manager_preview();
            state.manager_handle = Some(star_rail_worker::spawn_manager_preview(
                settings,
                state.manager_status.clone(),
                state.manager_preview.clone(),
            ));
        }

        if let Some(status) = worker::try_task_status(&state.manager_status) {
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
    if let Some(TaskStatus::Failed(error)) = worker::try_task_status(&state.manager_status) {
        widgets::error_card(ui, lang, &error);
    }
    ui.label(
        egui::RichText::new(lang.t(
            "先完整重扫并审核精确预览，再以预览摘要和逐类授权应用锁定或弃置标记。不会分解、删除或装备遗器。",
            "First rescan and review the exact preview. Lock or discard-mark changes can run only with that preview digest and per-scope authorization. Relics are never salvaged, deleted, or equipped.",
        ))
        .color(egui::Color32::from_rgb(120, 120, 120)),
    );
    ui.add_space(4.0);
    ui.separator();

    let preview = preview_snapshot(state);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::CollapsingHeader::new(lang.t("管理器文件", "Manager Files"))
                .default_open(true)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!is_running && !game_busy, |ui| {
                        path_row(
                            ui,
                            lang.t("GGStarRail 指令", "GGStarRail instructions"),
                            &mut settings.manager_instructions_path,
                            lang.t("选择文件...", "Choose file..."),
                            false,
                        );
                        journal_path_row(ui, lang, &mut settings.manager_journal_path);
                        ui.label(
                            egui::RichText::new(lang.t(
                                "恢复日志是追加写入的安全记录。若操作中断，请保留它并在下一次应用时使用同一路径。",
                                "The recovery journal is an append-only safety record. If an operation is interrupted, keep it and use the same path on the next apply.",
                            ))
                            .small()
                            .color(egui::Color32::from_rgb(120, 120, 120)),
                        );
                    });
                });

            egui::CollapsingHeader::new(lang.t("参考数据", "Reference Data"))
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!is_running && !game_busy, |ui| {
                        path_row(
                            ui,
                            lang.t("自定义参考数据（可选）", "Custom reference data (optional)"),
                            &mut settings.reference_bundle,
                            lang.t("选择覆盖文件夹...", "Choose override folder..."),
                            true,
                        );
                        ui.label(
                            egui::RichText::new(lang.t(
                                "默认使用程序内置且已验证的 GIlore 参考数据，无需另行下载。自定义覆盖仍必须完整，并与指令中的版本和修订完全一致。",
                                "The verified GIlore reference bundled with the app is used by default; no separate download is needed. A custom override must still be complete and exactly match the instructions' version and revision.",
                            ))
                            .small()
                            .color(egui::Color32::from_rgb(120, 120, 120)),
                        );
                    });
                });

            if let Some(preview) = preview {
                exact_preview(ui, lang, settings, state, preview, is_running, game_busy);
            } else if !is_running {
                ui.add_space(8.0);
                ui.colored_label(
                    egui::Color32::from_rgb(120, 120, 120),
                    lang.t(
                        "尚无可审核的预览。点击“完整重扫并生成预览”不会更改游戏。",
                        "There is no reviewable preview yet. Full Rescan and Preview never changes the game.",
                    ),
                );
            }
        });
}

fn invalidate_changed_preview(settings: &StarRailSettings, state: &mut StarRailState) {
    let identity = star_rail_worker::settings_identity(settings);
    let changed = match state.manager_preview.lock() {
        Ok(preview) => preview
            .as_ref()
            .is_some_and(|preview| preview.settings_identity != identity),
        Err(poisoned) => {
            state.manager_preview.clear_poison();
            poisoned
                .into_inner()
                .as_ref()
                .is_some_and(|preview| preview.settings_identity != identity)
        },
    };
    if changed {
        state.invalidate_manager_preview();
    }
}

fn preview_snapshot(state: &StarRailState) -> Option<ManagerPreview> {
    match state.manager_preview.lock() {
        Ok(preview) => preview.clone(),
        Err(poisoned) => {
            state.manager_preview.clear_poison();
            poisoned.into_inner().clone()
        },
    }
}

fn exact_preview(
    ui: &mut egui::Ui,
    lang: Lang,
    settings: &StarRailSettings,
    state: &mut StarRailState,
    preview: ManagerPreview,
    is_running: bool,
    game_busy: bool,
) {
    ui.add_space(8.0);
    ui.heading(lang.t("精确预览", "Exact Preview"));
    if preview.recovered {
        ui.colored_label(
            egui::Color32::from_rgb(255, 200, 50),
            lang.t(
                "这是从追加式恢复日志载入的原始预览。请重新审核并授权；应用前程序会重新读取游戏状态，不会盲目重复点击。",
                "This is the original preview recovered from the append-only journal. Review and authorize it again; before applying, the app rereads game state and never blindly repeats a click.",
            ),
        );
    }
    let rendered = preview.plan.render(match lang {
        Lang::Zh => Language::ZhCn,
        Lang::En => Language::En,
    });
    ui.add(
        egui::TextEdit::multiline(&mut rendered.as_str())
            .font(egui::TextStyle::Monospace)
            .desired_rows(10)
            .interactive(false),
    );
    egui::CollapsingHeader::new(lang.t("精确预览 JSON", "Exact Preview JSON"))
        .default_open(false)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut preview.exact_json.as_str())
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(12)
                    .interactive(false),
            );
        });

    ui.add_space(6.0);
    ui.label(lang.t("确认摘要：", "Confirmation digest:"));
    ui.monospace(&preview.plan.digest);

    let required = preview.plan.required_scopes();
    if required.is_empty() {
        ui.colored_label(
            egui::Color32::from_rgb(100, 200, 100),
            lang.t(
                "预览中没有需要应用的变更。",
                "The preview contains no changes to apply.",
            ),
        );
        return;
    }

    ui.add_space(6.0);
    ui.checkbox(
        &mut state.manager_reviewed,
        lang.t(
            "我已审核上方完整预览，并确认这是实时游戏状态变更",
            "I reviewed the complete preview above and confirm this is a live game-state change",
        ),
    );
    ui.label(lang.t(
        "逐类授权（只显示本次预览所需范围）：",
        "Per-scope authorization (only scopes required by this preview are shown):",
    ));
    if required.contains(&MutationScope::Lock) {
        ui.checkbox(&mut state.allow_lock, lang.t("允许锁定", "Allow locking"));
    }
    if required.contains(&MutationScope::Unlock) {
        ui.checkbox(
            &mut state.allow_unlock,
            lang.t("允许解锁", "Allow unlocking"),
        );
    }
    if required.contains(&MutationScope::MarkDiscard) {
        ui.checkbox(
            &mut state.allow_mark_discard,
            lang.t("允许标记弃置", "Allow marking discard"),
        );
    }
    if required.contains(&MutationScope::UnmarkDiscard) {
        ui.checkbox(
            &mut state.allow_unmark_discard,
            lang.t("允许取消弃置标记", "Allow removing discard marks"),
        );
    }

    let authorized = selected_scopes(state);
    let all_required = required.is_subset(&authorized);
    if ui
        .add_enabled(
            !is_running
                && !game_busy
                && state.manager_reviewed
                && all_required
                && !settings.manager_journal_path.trim().is_empty(),
            egui::Button::new(lang.t(
                "应用这份预览中的已授权变更",
                "Apply Authorized Changes from This Preview",
            )),
        )
        .clicked()
    {
        state.manager_handle = Some(star_rail_worker::spawn_manager_apply(
            settings,
            state.manager_status.clone(),
            state.manager_preview.clone(),
            authorized,
        ));
    }
    if !all_required {
        ui.colored_label(
            egui::Color32::from_rgb(255, 200, 50),
            lang.t(
                "请勾选预览所需的每一项授权。",
                "Select every authorization required by the preview.",
            ),
        );
    }
}

fn selected_scopes(state: &StarRailState) -> BTreeSet<MutationScope> {
    let mut scopes = BTreeSet::new();
    if state.allow_lock {
        scopes.insert(MutationScope::Lock);
    }
    if state.allow_unlock {
        scopes.insert(MutationScope::Unlock);
    }
    if state.allow_mark_discard {
        scopes.insert(MutationScope::MarkDiscard);
    }
    if state.allow_unmark_discard {
        scopes.insert(MutationScope::UnmarkDiscard);
    }
    scopes
}

fn journal_path_row(ui: &mut egui::Ui, lang: Lang, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(format!("{}:", lang.t("恢复日志", "Recovery journal")));
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width((ui.available_width() - 120.0).max(120.0)),
        );
        if ui.button(lang.t("选择路径...", "Choose path...")).clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("JSON Lines", &["jsonl"])
                .set_file_name("hsr_manager_journal.jsonl")
                .save_file()
            {
                *value = path.display().to_string();
            }
        }
    });
}
