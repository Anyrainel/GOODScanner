use std::sync::Arc;

use eframe::egui;

use super::state::{Lang, LogStore};

/// Show the log panel with explicit parameters.
pub fn show_with(ui: &mut egui::Ui, l: Lang, log_lines: &Arc<LogStore>) {
    // Build the full text once — shared between the Copy button and the TextEdit.
    let lines = log_lines.snapshot();
    let (count, full_text) = {
        let mut s = String::new();
        for entry in lines.iter() {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(&format!("{} {}", entry.timestamp, entry.message));
        }
        (lines.len(), s)
    };

    ui.horizontal(|ui| {
        ui.strong(l.t("日志", "Log"));
        if count > 0 {
            ui.colored_label(
                egui::Color32::from_rgb(120, 120, 120),
                format!("({})", count),
            );
            if ui.small_button(l.t("复制", "Copy")).clicked() {
                ui.ctx().copy_text(full_text.clone());
            }
            if ui.small_button(l.t("清除", "Clear")).clicked() {
                log_lines.clear();
            }
        }
    });

    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut full_text.as_str())
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(1),
            );
        });
}
