use eframe::egui::{self, Color32, RichText, Stroke};

use super::state::{Lang, TaskStatus};

pub const BACKGROUND: Color32 = Color32::from_rgb(19, 23, 28);
pub const SURFACE: Color32 = Color32::from_rgb(28, 33, 40);
pub const BORDER: Color32 = Color32::from_rgb(51, 61, 73);
pub const ACCENT: Color32 = Color32::from_rgb(124, 220, 192);
pub const MUTED: Color32 = Color32::from_rgb(165, 177, 192);
pub const SELECTED: Color32 = Color32::from_rgb(35, 55, 51);
pub const ERROR: Color32 = Color32::from_rgb(244, 132, 137);
pub const WARNING: Color32 = Color32::from_rgb(238, 190, 110);

pub fn setup(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BACKGROUND;
    style.visuals.window_fill = SURFACE;
    style.visuals.faint_bg_color = SURFACE;
    style.visuals.extreme_bg_color = BACKGROUND;
    style.visuals.override_text_color = Some(Color32::from_rgb(237, 242, 247));
    style.visuals.selection.bg_fill = Color32::from_rgb(38, 61, 53);
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    style.visuals.error_fg_color = ERROR;
    style.visuals.warn_fg_color = WARNING;
    // Panes have independent clipping. Keep every interaction state within the
    // widget's allocated bounds instead of egui's default 1px hover/press growth.
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.expansion = 0.0;
    }
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.noninteractive,
    ] {
        widget.bg_fill = SURFACE;
        widget.weak_bg_fill = SURFACE;
        widget.bg_stroke = Stroke::new(1.0, BORDER);
        widget.rounding = egui::Rounding::same(7.0);
    }
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(42, 51, 61);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(42, 51, 61);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    style.visuals.widgets.hovered.rounding = egui::Rounding::same(7.0);
    style.visuals.widgets.active.rounding = egui::Rounding::same(7.0);
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.scroll.bar_width = 6.0;
    style.spacing.scroll.floating = false;
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(23.0));
    ctx.set_style(style);
}

pub fn primary_action(ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
    let available = ui.available_width();
    let width = (available * 0.9).min(300.0);
    ui.allocate_ui_with_layout(
        egui::vec2(available, 44.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.visuals_mut().widgets.inactive.weak_bg_fill = ACCENT;
            ui.visuals_mut().widgets.noninteractive.weak_bg_fill = ACCENT;
            ui.visuals_mut().widgets.hovered.weak_bg_fill = Color32::from_rgb(155, 234, 211);
            ui.visuals_mut().widgets.active.weak_bg_fill = Color32::from_rgb(108, 203, 177);
            ui.add_enabled(
                enabled,
                egui::Button::new(RichText::new(text).color(Color32::from_rgb(16, 41, 31)))
                    .stroke(Stroke::NONE)
                    .rounding(8.0)
                    .min_size(egui::vec2(width, 42.0)),
            )
        },
    )
    .inner
}

/// A quiet native button with a visible hover fill, border, and brighter text.
pub fn text_button(
    ui: &mut egui::Ui,
    text: &str,
    size: egui::Vec2,
    selected: bool,
) -> egui::Response {
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        visuals.override_text_color = None;
        visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        visuals.widgets.inactive.bg_stroke = Stroke::NONE;
        visuals.widgets.inactive.fg_stroke.color = if selected { ACCENT } else { MUTED };
        visuals.widgets.hovered.fg_stroke.color = ACCENT;
        visuals.widgets.active.fg_stroke.color = ACCENT;
        ui.add_sized(size, egui::Button::new(text))
    })
    .inner
}

pub fn task_status(ui: &mut egui::Ui, lang: Lang, status: Option<&TaskStatus>, idle: &str) {
    ui.label(RichText::new(lang.t("任务状态", "Task status")).color(MUTED));
    ui.add_space(4.0);
    match status {
        Some(TaskStatus::Running(message)) => {
            ui.add(egui::Label::new(RichText::new(message.text(lang)).size(18.0)).wrap())
                .on_hover_text(message.text(lang));
        },
        Some(TaskStatus::Completed(message))
        | Some(TaskStatus::Stopped(message))
        | Some(TaskStatus::AwaitingInput(message)) => {
            let color = if matches!(
                status,
                Some(TaskStatus::Stopped(_) | TaskStatus::AwaitingInput(_))
            ) {
                ui.visuals().warn_fg_color
            } else {
                ACCENT
            };
            ui.add(
                egui::Label::new(RichText::new(message.text(lang)).size(18.0).color(color)).wrap(),
            );
        },
        Some(TaskStatus::Exported { path, partial, .. }) => {
            let title = if *partial {
                lang.t("部分结果已导出", "Partial results saved")
            } else {
                lang.t("数据已导出", "Export saved")
            };
            ui.add(
                egui::Label::new(RichText::new(title).size(18.0).color(if *partial {
                    ui.visuals().warn_fg_color
                } else {
                    ACCENT
                }))
                .wrap(),
            );
            super::widgets::export_file(ui, lang, path);
        },
        Some(TaskStatus::Failed(error)) => {
            ui.label(
                RichText::new(lang.t("任务未完成", "Task did not finish"))
                    .size(18.0)
                    .color(ui.visuals().error_fg_color),
            );
            super::widgets::error_card(ui, lang, error);
        },
        _ => {
            ui.add(egui::Label::new(RichText::new(idle).size(18.0)).wrap())
                .on_hover_text(idle);
        },
    }
    ui.add_space(12.0);
}

pub fn blocked_status(ui: &mut egui::Ui, lang: Lang, reason: &str) {
    task_status(
        ui,
        lang,
        Some(&TaskStatus::AwaitingInput(super::state::UiText::new(
            reason, reason,
        ))),
        "",
    );
}

pub fn restart_required(ui: &mut egui::Ui, lang: Lang, error: Option<&super::state::UiError>) {
    if let Some(error) = error {
        super::widgets::error_card(ui, lang, error);
    } else {
        ui.colored_label(
            ui.visuals().error_fg_color,
            lang.t(
                "任务发生内部错误，请重启程序。",
                "An internal task error requires restarting the app.",
            ),
        );
    }
    primary_action(ui, false, lang.t("需重启程序", "Restart required"));
}
