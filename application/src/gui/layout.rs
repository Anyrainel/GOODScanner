use eframe::egui;

use super::theme;

#[derive(Clone, Copy)]
pub enum Pane {
    Settings,
    Status,
}

/// Keep the title inset comfortable without wasting space above the log divider.
pub fn workspace_bounds(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        rect.min + egui::vec2(12.0, 12.0),
        rect.max - egui::vec2(12.0, 4.0),
    )
}

/// Fractions are session UI state, independent of task state and saved game settings.
pub struct Splits {
    pub sidebar: f32,
    pub workspace: f32,
    pub settings: f32,
}

impl Default for Splits {
    fn default() -> Self {
        Self {
            sidebar: 0.10,
            workspace: 0.80,
            settings: 0.50,
        }
    }
}

pub fn split(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: &str,
    fraction: &mut f32,
    horizontal: bool,
    minimum: f32,
) -> [egui::Rect; 2] {
    let size = if horizontal {
        rect.width()
    } else {
        rect.height()
    };
    let limit = (minimum / size.max(1.0)).min(0.45);
    let origin = if horizontal { rect.left() } else { rect.top() };
    let mut position = origin + size * fraction.clamp(limit, 1.0 - limit);
    let divider = if horizontal {
        egui::Rect::from_min_max(
            egui::pos2(position - 3.0, rect.top()),
            egui::pos2(position + 3.0, rect.bottom()),
        )
    } else {
        egui::Rect::from_min_max(
            egui::pos2(rect.left(), position - 3.0),
            egui::pos2(rect.right(), position + 3.0),
        )
    };
    let response = ui.interact(divider, ui.id().with(id), egui::Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(if horizontal {
            egui::CursorIcon::ResizeHorizontal
        } else {
            egui::CursorIcon::ResizeVertical
        });
    }
    if response.dragged() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let coordinate = if horizontal { pointer.x } else { pointer.y };
            *fraction = ((coordinate - origin) / size.max(1.0)).clamp(limit, 1.0 - limit);
            position = origin + size * *fraction;
        }
    }
    let color = if response.hovered() || response.dragged() {
        theme::ACCENT
    } else {
        theme::BORDER
    };
    let ends = if horizontal {
        [
            egui::pos2(position, rect.top()),
            egui::pos2(position, rect.bottom()),
        ]
    } else {
        [
            egui::pos2(rect.left(), position),
            egui::pos2(rect.right(), position),
        ]
    };
    ui.painter()
        .line_segment(ends, egui::Stroke::new(1.0, color));
    if horizontal {
        [
            egui::Rect::from_min_max(rect.min, egui::pos2(position - 3.0, rect.bottom())),
            egui::Rect::from_min_max(egui::pos2(position + 3.0, rect.top()), rect.max),
        ]
    } else {
        [
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), position - 3.0)),
            egui::Rect::from_min_max(egui::pos2(rect.left(), position + 3.0), rect.max),
        ]
    }
}

pub fn region(ui: &mut egui::Ui, rect: egui::Rect, id: &str, render: impl FnOnce(&mut egui::Ui)) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    render(&mut child);
}

pub fn workspace(
    ui: &mut egui::Ui,
    fraction: &mut f32,
    mut render: impl FnMut(&mut egui::Ui, Pane),
) {
    let rect = ui.available_rect_before_wrap();
    let [left, right] = split(ui, rect, "settings-status", fraction, true, 180.0);
    for (pane, rect, id) in [
        (
            Pane::Settings,
            left.shrink2(egui::vec2(12.0, 0.0)),
            "settings",
        ),
        (Pane::Status, right.shrink2(egui::vec2(12.0, 0.0)), "status"),
    ] {
        region(ui, rect, id, |ui| {
            let content_width = (rect.width() - ui.spacing().scroll.allocated_width()).max(1.0);
            egui::ScrollArea::both()
                .id_salt(id)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(content_width);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    if matches!(pane, Pane::Settings) {
                        ui.spacing_mut().item_spacing.y = 10.0;
                        render(ui, pane);
                        return;
                    }
                    egui::Frame::none()
                        .fill(theme::SURFACE)
                        .stroke(egui::Stroke::new(1.0, theme::BORDER))
                        .rounding(14.0)
                        .inner_margin(16.0)
                        .outer_margin(2.0)
                        .show(ui, |ui| {
                            ui.set_width((content_width - 36.0).max(1.0));
                            render(ui, pane);
                        });
                });
        });
    }
}
