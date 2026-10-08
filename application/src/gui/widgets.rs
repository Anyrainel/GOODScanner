//! Shared UI widgets used by both scanner and manager tabs.

use eframe::egui;

use super::state::{AppState, Lang, UiError};

/// Native egui frames provide the same spacing and hierarchy for every form.
pub fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    form_frame(ui, |ui| {
        ui.label(egui::RichText::new(title).size(13.0).strong());
        ui.add_space(3.0);
        body(ui);
    });
}

pub fn fold(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    form_frame(ui, |ui| {
        egui::CollapsingHeader::new(egui::RichText::new(title).size(13.0).strong())
            .show_unindented(ui, |ui| {
                ui.add_space(3.0);
                body(ui);
            });
    });
}

fn form_frame(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    egui::Frame::none()
        .fill(super::theme::SURFACE)
        .rounding(9.0)
        .inner_margin(egui::vec2(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_width((width - 24.0).max(1.0));
            ui.style_mut().override_font_id = Some(egui::FontId::proportional(13.0));
            ui.spacing_mut().item_spacing = egui::vec2(7.0, 4.0);
            ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
            ui.spacing_mut().interact_size.y = 26.0;
            body(ui);
        });
}

/// Labels and controls share a fixed-height row and a stable label column.
pub fn field_row(ui: &mut egui::Ui, label: &str, control: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    let label_width = (width * 0.35).clamp(65.0, 112.0);
    let (row, _) = ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::hover());
    let label_rect =
        egui::Rect::from_min_max(row.min, egui::pos2(row.left() + label_width, row.bottom()));
    let control_rect =
        egui::Rect::from_min_max(egui::pos2(label_rect.right() + 8.0, row.top()), row.max);
    let mut label_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(label_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    label_ui
        .add(egui::Label::new(label).truncate())
        .on_hover_text(label);
    let mut control_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(control_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    control(&mut control_ui);
}

pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .size(12.0)
                .color(super::theme::MUTED),
        )
        .wrap(),
    );
}

/// Keep text, selection and caret centered when a form gives the input extra height.
pub fn singleline_input(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).vertical_align(egui::Align::Center)
}

/// Both games expose the same export choices with identical copy and order.
pub fn scan_export_options(
    ui: &mut egui::Ui,
    lang: Lang,
    only_latest: &mut bool,
    save_on_cancel: &mut bool,
) {
    ui.checkbox(
        only_latest,
        lang.t("仅保留最新导出", "Keep latest export only"),
    );
    ui.checkbox(
        save_on_cancel,
        lang.t("停止时保存已扫描结果", "Save results on stop"),
    );
}

/// Render every user-visible failure with the same information hierarchy:
/// a localized hint first, then copyable diagnostics behind a native disclosure.
pub fn error_card(ui: &mut egui::Ui, l: Lang, error: &UiError) {
    let hint = error.hint_text(l);
    let technical_details = error.technical_details(l);
    let copy_text = error.copy_text(l);

    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.colored_label(
            ui.visuals().error_fg_color,
            egui::RichText::new(hint).strong(),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .small_button(l.t("复制完整错误", "Copy full error"))
                .clicked()
            {
                ui.ctx().copy_text(copy_text.clone());
            }
        });
        egui::CollapsingHeader::new(l.t("错误详情", "Error details")).show_unindented(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut technical_details.as_str())
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(technical_details.lines().count().clamp(2, 6)),
            );
        });
    });
}

pub fn export_file(ui: &mut egui::Ui, l: Lang, path: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(l.t("导出文件", "Export file"))
                .size(12.0)
                .color(super::theme::MUTED),
        );
        if ui.small_button(l.t("复制路径", "Copy path")).clicked() {
            ui.ctx().copy_text(path.to_owned());
        }
    });
    ui.add(
        singleline_input(&mut path.as_ref())
            .font(egui::TextStyle::Small)
            .desired_width(ui.available_width()),
    );
}

/// Numeric input for u64 values (clamped to 5000).
pub fn num_input_u64(ui: &mut egui::Ui, value: &mut u64, width: f32) {
    ui.add_sized(
        [width, 24.0],
        egui::DragValue::new(value).range(0..=5000).speed(1.0),
    );
}

/// A labeled group of delay fields with aligned value controls.
/// Each field is `(label, value, default, tooltip)`. Values below their default get a
/// warning asterisk, and a footnote is shown underneath when any field is below default.
pub fn delay_group(
    ui: &mut egui::Ui,
    id: &str,
    category: &str,
    l: Lang,
    fields: &mut [(&str, &mut u64, u64, &str)],
) {
    ui.strong(format!("{category} (ms)"));
    let mut any_below = false;
    let warn_color = egui::Color32::from_rgb(255, 200, 50);
    ui.push_id(id, |ui| {
        for (label, value, default, tooltip) in fields.iter_mut() {
            let below = **value < *default;
            let label_text = if below {
                any_below = true;
                format!("{}*", label)
            } else {
                label.to_string()
            };
            let rich = if below {
                egui::RichText::new(&label_text).color(warn_color)
            } else {
                egui::RichText::new(&label_text)
            };
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    num_input_u64(ui, value, 60.0);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(rich).truncate())
                            .on_hover_text(format!("{label}\n{tooltip}"));
                    });
                });
            });
        }
    });
    if any_below {
        ui.colored_label(
            warn_color,
            l.t(
                "* 低于默认值，可能导致扫描不稳定",
                "* Below default — may cause unreliable scans",
            ),
        );
    }
}

/// Character names section — shared between scanner and manager tabs.
/// Shows the 4 renameable character name fields with required-field validation.
pub fn character_names_section(ui: &mut egui::Ui, state: &mut AppState, enabled: bool) {
    let l = state.lang;

    ui.add_enabled_ui(enabled, |ui| {
        if state.names_need_attention {
            ui.colored_label(
                egui::Color32::from_rgb(255, 200, 50),
                l.t(
                    "请填写必填角色名称（旅行者），然后再次点击开始扫描。",
                    "Fill in the required name (Traveler), then click Start Scan again.",
                ),
            );
            ui.add_space(4.0);
        } else {
            hint(
                ui,
                l.t(
                    "填写游戏内昵称；* 为必填。",
                    "Use in-game names. * Required.",
                ),
            );
        }
        field_row(ui, l.t("旅行者*", "Traveler*"), |ui| {
            ui.add(
                singleline_input(&mut state.user_config.traveler_name)
                    .desired_width(ui.available_width())
                    .min_size(egui::vec2(0.0, 26.0)),
            );
        });
        egui::CollapsingHeader::new(l.t("其他可改名角色", "Other renamed characters"))
            .show_unindented(ui, |ui| {
                for (label, name) in [
                    (
                        l.t("流浪者", "Wanderer"),
                        &mut state.user_config.wanderer_name,
                    ),
                    (
                        l.t("奇偶·男性", "Manekin"),
                        &mut state.user_config.manekin_name,
                    ),
                    (
                        l.t("奇偶·女性", "Manekina"),
                        &mut state.user_config.manekina_name,
                    ),
                ] {
                    field_row(ui, label, |ui| {
                        ui.add(
                            singleline_input(name)
                                .desired_width(ui.available_width())
                                .min_size(egui::vec2(0.0, 26.0)),
                        );
                    });
                }
            });

        if state.names_need_attention && !state.missing_required_character_names() {
            state.names_need_attention = false;
        }
    });
}

/// Inventory delay fields — shared between scanner and manager tabs.
/// Renders as a delay_group with the 7 inventory timing fields.
pub fn inventory_delays(ui: &mut egui::Ui, state: &mut AppState, l: Lang) {
    let defaults = genshin_scanner::cli::GoodUserConfig::default();
    delay_group(ui, "inv_delays", l.t("背包", "Inventory"), l, &mut [
        (l.t("打开背包", "Open backpack"), &mut state.user_config.inv_open_delay, defaults.inv_open_delay,
            l.t("按下快捷键后等待背包界面完全加载的时间", "Wait time after pressing hotkey for backpack to fully load")),
        (l.t("标签切换", "Tab switch"), &mut state.user_config.inv_tab_delay, defaults.inv_tab_delay,
            l.t("切换武器/圣遗物标签后等待内容加载的时间", "Wait time after switching weapon/artifact tab for content to load")),
        (l.t("翻页等待", "Page scroll"), &mut state.user_config.inv_scroll_delay, defaults.inv_scroll_delay,
            l.t("翻页后等待物品列表稳定的时间", "Wait time after scrolling for item list to stabilize")),
        (l.t("武器面板延迟", "Weapon panel delay"), &mut state.user_config.weapon_panel_delay, defaults.weapon_panel_delay,
            l.t("点击武器后固定等待时间，然后检查面板稳定", "Fixed wait after clicking a weapon, then verify panel is stable")),
        (l.t("圣遗物初始等待", "Artifact initial wait"), &mut state.user_config.artifact_initial_wait, defaults.artifact_initial_wait,
            l.t("点击圣遗物后、开始检测面板变化前的最小等待", "Minimum wait after clicking an artifact before starting panel change detection")),
        (l.t("圣遗物面板超时", "Artifact panel timeout"), &mut state.user_config.artifact_panel_timeout, defaults.artifact_panel_timeout,
            l.t("等待圣遗物面板内容变化的最大时间，超时则直接截图", "Max time to wait for artifact panel content to change; captures on timeout")),
        (l.t("圣遗物额外延迟", "Artifact extra delay"), &mut state.user_config.artifact_extra_delay, defaults.artifact_extra_delay,
            l.t("面板加载完成后、截图前的额外等待（通常为0）", "Extra wait after panel loaded before capturing (usually 0)")),
    ]);
}
