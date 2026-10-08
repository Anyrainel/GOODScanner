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

    ui.spacing_mut().item_spacing.y = 4.0;
    ui.spacing_mut().button_padding = egui::vec2(6.0, 2.0);
    ui.spacing_mut().interact_size.y = 20.0;
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

    let body = ui.available_rect_before_wrap();
    let content_width = (body.width() - ui.spacing().scroll.allocated_width()).max(1.0);
    let content_height = (body.height() - ui.spacing().scroll.allocated_width()).max(1.0);
    egui::ScrollArea::both()
        .id_salt("log-scroll")
        .auto_shrink([false; 2])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            // Preserve unwrapped log lines and text selection, while letting
            // the read-only field fill the log pane even before the first log.
            let mut layouter = |ui: &egui::Ui, _: &str, _: f32| {
                let mut job = egui::text::LayoutJob::default();
                job.wrap.max_width = f32::INFINITY;
                for (index, entry) in lines.iter().enumerate() {
                    let color = match entry.level {
                        log::Level::Error => ui.visuals().error_fg_color,
                        log::Level::Warn => ui.visuals().warn_fg_color,
                        _ => ui.visuals().text_color(),
                    };
                    job.append(
                        &format!(
                            "{}{} {}",
                            if index == 0 { "" } else { "\n" },
                            entry.timestamp,
                            entry.message
                        ),
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::monospace(12.0),
                            color,
                            ..Default::default()
                        },
                    );
                }
                ui.fonts(|fonts| fonts.layout_job(job))
            };
            ui.add(
                egui::TextEdit::multiline(&mut full_text.as_str())
                    .font(egui::TextStyle::Monospace)
                    .frame(false)
                    .layouter(&mut layouter)
                    .desired_width(content_width)
                    .min_size(egui::vec2(content_width, content_height))
                    .desired_rows(1),
            );
        });
}
