use eframe::egui;

use super::{
    state::Lang,
    task_progress::{self, Step, StepState},
    widgets,
};

/// Both capture adapters report the same user journey from their actual monitor state.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    Ready,
    Starting,
    Login,
    Data,
    Achievements,
    Stopping,
    Exporting,
    Done,
    Stopped,
    Failed,
}

impl Stage {
    pub fn headline(self, lang: Lang) -> &'static str {
        match self {
            Self::Ready => lang.t("请先关闭游戏", "Close the game first"),
            Self::Starting => lang.t("正在准备抓包", "Preparing capture"),
            Self::Login => lang.t("请启动游戏并登录", "Launch the game and log in"),
            Self::Data => lang.t("正在等待剩余类别数据", "Waiting for remaining data"),
            Self::Achievements => lang.t("请在游戏中打开成就", "Open Achievements in the game"),
            Self::Stopping => lang.t("正在停止抓包", "Stopping capture"),
            Self::Exporting => lang.t("正在导出数据", "Exporting data"),
            Self::Done => lang.t("数据已导出", "Data exported"),
            Self::Stopped => lang.t("抓包已停止 · 未导出", "Capture stopped · no export"),
            Self::Failed => lang.t("抓包未完成", "Capture failed"),
        }
    }
}

pub(super) fn waiting(
    connected: bool,
    missing_inventory: bool,
    missing_achievements: bool,
) -> Stage {
    if !connected {
        Stage::Login
    } else if !missing_inventory && missing_achievements {
        Stage::Achievements
    } else {
        Stage::Data
    }
}

pub(super) fn show_connection(ui: &mut egui::Ui, lang: Lang, stage: Stage, connected: bool) {
    if stage == Stage::Done {
        widgets::hint(
            ui,
            lang.t(
                "在 GGArtifact 中导入上面的文件。",
                "Import the file above into GGArtifact.",
            ),
        );
    } else if stage == Stage::Login {
        widgets::hint(
            ui,
            lang.t(
                "游戏已在运行？请关闭后重新登录。",
                "Already in-game? Close it and log in again.",
            ),
        );
    }
    let mut step = Step::new("login", "登录数据", "Login data");
    step.show_count = false;
    step.state = if connected {
        StepState::Complete
    } else if matches!(stage, Stage::Stopped | Stage::Failed) {
        StepState::Interrupted
    } else if matches!(stage, Stage::Starting | Stage::Login) {
        StepState::Running
    } else {
        StepState::Pending
    };
    task_progress::row(ui, lang, &step);
}

#[cfg(test)]
pub(crate) mod feedback_tests {
    use super::*;

    #[test]
    fn login_step_never_shows_a_meaningless_item_count() {
        let content = texts(|ui| show_connection(ui, Lang::En, Stage::Login, false));
        assert!(content.iter().any(|text| text == "Login data"));
        assert!(!content.iter().any(|text| text == "0"));
    }

    pub(crate) fn texts(mut draw: impl FnMut(&mut egui::Ui)) -> Vec<String> {
        fn collect(shape: &egui::Shape, texts: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => texts.push(text.galley.job.text.clone()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, texts)),
                _ => {},
            }
        }
        let ctx = egui::Context::default();
        super::super::theme::setup(&ctx);
        let mut texts = Vec::new();
        for _ in 0..2 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(370.0, 700.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| draw(ui));
                },
            );
            texts.clear();
            for clipped in output.shapes {
                collect(&clipped.shape, &mut texts);
            }
        }
        texts
    }
}
