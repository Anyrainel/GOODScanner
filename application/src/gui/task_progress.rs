use super::{state::Lang, theme};
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Pending,
    Running,
    Complete,
    Interrupted,
}

#[derive(Clone, Debug)]
pub struct Step {
    pub key: &'static str,
    pub zh: &'static str,
    pub en: &'static str,
    pub completed: usize,
    pub total: Option<usize>,
    pub state: StepState,
}

impl Step {
    pub fn new(key: &'static str, zh: &'static str, en: &'static str) -> Self {
        Self {
            key,
            zh,
            en,
            completed: 0,
            total: None,
            state: StepState::Pending,
        }
    }
}

#[derive(Clone, Default)]
pub struct TaskProgress {
    pub steps: Vec<Step>,
}

impl TaskProgress {
    pub fn interrupt_unfinished(&mut self) {
        for step in &mut self.steps {
            if matches!(step.state, StepState::Pending | StepState::Running) {
                step.state = StepState::Interrupted;
            }
        }
    }
}

pub fn show(ui: &mut egui::Ui, lang: Lang, progress: &TaskProgress) {
    for step in &progress.steps {
        row(ui, lang, step);
    }
}

pub fn row(ui: &mut egui::Ui, lang: Lang, step: &Step) {
    let color = match step.state {
        StepState::Complete => theme::ACCENT,
        StepState::Running => ui.visuals().text_color(),
        StepState::Interrupted => ui.visuals().warn_fg_color,
        StepState::Pending => theme::MUTED,
    };
    let count = if step.state != StepState::Pending || step.total.is_some() {
        match step.total {
            Some(total) => format!("{} / {total}", step.completed),
            None => step.completed.to_string(),
        }
    } else {
        String::new()
    };
    // Allocate the entire row once: counts and meters never introduce a second line.
    let width = ui.available_width().max(180.0);
    let compact = width < 290.0;
    let bar_width = (width * 0.27).clamp(44.0, 96.0);
    let count_width = if compact { 68.0 } else { 86.0 };
    let gap = 6.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::hover());
    let icon = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 6.0, rect.center().y),
        egui::vec2(12.0, 12.0),
    );
    match step.state {
        StepState::Complete => {
            ui.painter().line_segment(
                [
                    icon.left_center() + egui::vec2(1.0, 0.0),
                    icon.center() + egui::vec2(-1.0, 3.0),
                ],
                egui::Stroke::new(1.6, color),
            );
            ui.painter().line_segment(
                [
                    icon.center() + egui::vec2(-1.0, 3.0),
                    icon.right_top() + egui::vec2(-1.0, 1.0),
                ],
                egui::Stroke::new(1.6, color),
            );
        },
        StepState::Interrupted => {
            ui.painter().text(
                icon.center(),
                egui::Align2::CENTER_CENTER,
                "!",
                egui::FontId::proportional(13.0),
                color,
            );
            ui.interact(icon, ui.id().with(step.key), egui::Sense::hover())
                .on_hover_text(
                    lang.t("该步骤未完整完成。", "This step did not finish completely."),
                );
        },
        StepState::Running => {
            ui.painter().circle_filled(icon.center(), 3.0, color);
        },
        StepState::Pending => {
            ui.painter()
                .circle_stroke(icon.center(), 3.0, egui::Stroke::new(1.0, color));
        },
    }
    let bar = egui::Rect::from_center_size(
        egui::pos2(rect.right() - bar_width / 2.0, rect.center().y),
        egui::vec2(bar_width, 4.0),
    );
    let numbers = egui::Rect::from_min_max(
        egui::pos2(bar.left() - gap - count_width, rect.top()),
        egui::pos2(bar.left() - gap, rect.bottom()),
    );
    let name = egui::Rect::from_min_max(
        egui::pos2(icon.right() + gap, rect.top()),
        egui::pos2(numbers.left() - gap, rect.bottom()),
    );
    let label = if compact && step.key == "light_cones" {
        lang.t("光锥", "Cones")
    } else if step.key == "gear" || step.key == "relics" {
        lang.t("遗器与饰品", "Relics")
    } else {
        lang.t(step.zh, step.en)
    };
    let mut name_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(name)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    name_ui
        .add(egui::Label::new(egui::RichText::new(label).color(color).size(13.0)).truncate())
        .on_hover_text(lang.t(step.zh, step.en));
    let mut count_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(numbers)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let display_count = if compact {
        count.replace(" / ", "/")
    } else {
        count.clone()
    };
    count_ui
        .add(
            egui::Label::new(
                egui::RichText::new(display_count)
                    .color(color)
                    .size(if compact { 11.0 } else { 12.0 }),
            )
            .truncate(),
        )
        .on_hover_text(count);
    let mut bar_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    meter(&mut bar_ui, step, color, bar_width);
}

fn meter(ui: &mut egui::Ui, step: &Step, color: egui::Color32, width: f32) {
    if let Some(total) = step.total.filter(|&t| t > 0) {
        ui.add(
            egui::ProgressBar::new((step.completed as f32 / total as f32).clamp(0.0, 1.0))
                .desired_width(width)
                .desired_height(4.0)
                .fill(color),
        );
    } else {
        ui.allocate_space(egui::vec2(width, 4.0));
    }
}
