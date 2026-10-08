use eframe::egui::{self, epaint::Shape, Color32};
use good_tools_app::gui::{
    log_panel, scanner_tab, star_rail_scanner_tab,
    star_rail_state::StarRailState,
    state::{AppState, Lang, LogEntry, LogSource, LogStore, TaskStatus, UiError, UiText},
    theme,
};
use std::sync::Arc;

fn text_shapes(shape: &Shape, out: &mut Vec<(String, f32, Color32)>) {
    match shape {
        Shape::Text(text) => {
            for section in &text.galley.job.sections {
                out.push((
                    text.galley.job.text[section.byte_range.clone()].into(),
                    text.pos.y,
                    section.format.color,
                ));
            }
        },
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| text_shapes(shape, out)),
        _ => {},
    }
}

fn render(mut draw: impl FnMut(&mut egui::Ui)) -> Vec<(String, f32, Color32)> {
    let ctx = egui::Context::default();
    theme::setup(&ctx);
    let mut texts = Vec::new();
    for _ in 0..2 {
        let frame = ctx.run(
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
        for clipped in &frame.shapes {
            text_shapes(&clipped.shape, &mut texts);
        }
    }
    texts
}

#[test]
fn failure_is_prominent_before_retry_without_expanding_technical_clutter() {
    let status = TaskStatus::Failed(UiError::from_message(
        UiText::new("输出目录无法写入。", "The output folder is not writable."),
        "RAW_DIAGNOSTIC_MARKER: os error 5",
    ));
    let texts = render(|ui| {
        theme::task_status(ui, Lang::En, Some(&status), "Ready");
        theme::primary_action(ui, true, "Retry");
    });
    let hint = texts
        .iter()
        .find(|(text, _, _)| text == "The output folder is not writable.")
        .unwrap();
    let retry = texts.iter().find(|(text, _, _)| text == "Retry").unwrap();
    assert!(hint.1 < retry.1);
    assert_eq!(hint.2, theme::ERROR);
    assert!(texts.iter().any(|(text, _, _)| text == "Copy full error"));
    assert!(!texts
        .iter()
        .any(|(text, _, _)| text.contains("RAW_DIAGNOSTIC_MARKER")));
}

#[test]
fn export_path_is_visible_only_after_a_successful_write() {
    let path = "C:/exports/scan.json";
    for partial in [false, true] {
        let status = TaskStatus::Exported {
            message: UiText::new("完成", "Done"),
            path: path.into(),
            partial,
        };
        let texts = render(|ui| theme::task_status(ui, Lang::En, Some(&status), "Ready"));
        let title = if partial {
            "Partial results saved"
        } else {
            "Export saved"
        };
        let title = texts.iter().find(|(text, _, _)| text == title).unwrap();
        assert_eq!(
            title.2,
            if partial {
                theme::WARNING
            } else {
                theme::ACCENT
            }
        );
        assert!(texts.iter().any(|(text, _, _)| text == path));
        assert!(texts.iter().any(|(text, _, _)| text == "Copy path"));
    }
    let status = TaskStatus::Running(UiText::new("正在导出", "Saving export"));
    let texts = render(|ui| theme::task_status(ui, Lang::En, Some(&status), "Ready"));
    assert!(!texts
        .iter()
        .any(|(text, _, _)| text == "Copy path" || text == "Export saved"));
}

#[test]
fn missing_scan_targets_explain_why_start_is_disabled() {
    let mut state = AppState::new();
    state.lang = Lang::En;
    state.scan_characters = false;
    state.scan_weapons = false;
    state.scan_artifacts = false;
    state.scan_achievements = false;
    let mut handle = None;
    let texts = render(|ui| scanner_tab::show_status(ui, &mut state, &mut handle, false, false));
    assert!(texts
        .iter()
        .any(|(text, _, _)| text == "Select scan targets"));
    assert!(!texts.iter().any(|(text, _, _)| text == "Ready to scan"));
}

#[test]
fn fixing_required_input_clears_the_old_blocking_status() {
    let mut state = AppState::new();
    state.lang = Lang::En;
    state.user_config.traveler_name = "Traveler".into();
    *state.scan_status.lock().unwrap() =
        TaskStatus::AwaitingInput(UiText::new("请填写旅行者名字", "Enter the Traveler's name"));
    let mut handle = None;
    let texts = render(|ui| scanner_tab::show_status(ui, &mut state, &mut handle, false, false));
    assert!(texts.iter().any(|(text, _, _)| text == "Ready to scan"));
    assert!(!texts
        .iter()
        .any(|(text, _, _)| text == "Enter the Traveler's name"));
}

#[test]
fn required_star_rail_inputs_replace_ready_until_they_are_supplied() {
    let mut settings = good_tools_app::config::StarRailSettings::default();
    let mut state = StarRailState::new(String::new());
    settings.scan_characters = true;
    settings.output_dir = "C:/exports".into();
    let texts = render(|ui| {
        star_rail_scanner_tab::show_status(ui, Lang::En, &mut settings, &mut state, false, false)
    });
    assert!(!texts.iter().any(|(text, _, _)| text == "Ready to scan"));
    settings.trailblazer_name = "Trailblazer".into();
    settings.trailblazer_gender = Some(hsr_scanner::TrailblazerGender::Stelle);
    let texts = render(|ui| {
        star_rail_scanner_tab::show_status(ui, Lang::En, &mut settings, &mut state, false, false)
    });
    assert!(texts.iter().any(|(text, _, _)| text == "Ready to scan"));
}

#[test]
fn compact_logs_preserve_warning_and_error_emphasis() {
    let store = Arc::new(LogStore::new(100));
    for (level, message) in [
        (log::Level::Warn, "WARN_MARKER"),
        (log::Level::Error, "ERROR_MARKER"),
    ] {
        store.push(LogEntry {
            level,
            message: message.into(),
            timestamp: "12:00".into(),
            source: LogSource::Scanner,
        });
    }
    let texts = render(|ui| log_panel::show_with(ui, Lang::En, &store));
    assert!(texts
        .iter()
        .any(|(text, _, color)| text.contains("WARN_MARKER") && *color == theme::WARNING));
    assert!(texts
        .iter()
        .any(|(text, _, color)| text.contains("ERROR_MARKER") && *color == theme::ERROR));
}

#[cfg(feature = "capture")]
#[test]
fn failed_capture_write_displays_the_cause_and_never_shows_an_export_path() {
    use good_tools_app::gui::capture_tab::{self, CaptureTabState};
    let missing = std::env::temp_dir()
        .join(format!(
            "ggscanner-feedback-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ))
        .join("missing-output");
    let mut capture = CaptureTabState::new(missing.display().to_string());
    capture.only_keep_latest_dump = false;
    let export = genshin_scanner::scanner::common::models::GoodExport::new(
        Some(vec![]),
        Some(vec![]),
        Some(vec![]),
    );
    capture.inject_export_for_test(export).send(()).unwrap();
    for _ in 0..200 {
        capture.tick();
        if !capture.is_busy() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!capture.is_busy());
    let texts = render(|ui| capture_tab::show_status(ui, Lang::En, &mut capture, false, false));
    assert!(texts
        .iter()
        .any(|(text, _, _)| text.contains("export file could not be written")));
    assert!(texts.iter().any(|(text, _, _)| text == "Retry"));
    assert!(!texts
        .iter()
        .any(|(text, _, _)| text == "Copy path" || text == "Export saved"));
}
