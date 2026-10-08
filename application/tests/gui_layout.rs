use eframe::egui::{self, Event, Modifiers, PointerButton};
use good_tools_app::gui::{
    hsr_manager_progress::{plan_steps, TrackedJournal},
    layout,
    state::{TaskStatus, UiText},
    task_progress::{StepState, TaskProgress},
    theme,
};
use hsr_scanner::manager::{JournalEntry, JournalStatus, ManagerJournal, ManagerJournalStore};
use std::sync::{Arc, Mutex};

#[test]
fn progress_rows_keep_the_same_height_at_narrow_and_wide_widths() {
    use good_tools_app::gui::{
        state::Lang,
        task_progress::{self, Step},
    };
    let ctx = egui::Context::default();
    theme::setup(&ctx);
    for width in [180.0, 220.0, 360.0] {
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                layout::region(
                    ui,
                    egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(width, 400.0)),
                    "progress-size",
                    |ui| {
                        let mut previous_row_top = None;
                        for (key, label, count) in [
                            ("characters", "Characters", 86),
                            ("weapons", "Weapons", 247),
                            ("artifacts", "Artifacts", 1342),
                        ] {
                            let mut step = Step::new(key, label, label);
                            step.completed = count;
                            step.state = StepState::Complete;
                            if key != "characters" {
                                step.total = Some(count);
                            }
                            let row_top = ui.cursor().top();
                            task_progress::row(ui, Lang::En, &step);
                            if let Some(previous) = previous_row_top {
                                assert_eq!(row_top - previous, 28.0 + ui.spacing().item_spacing.y);
                            }
                            previous_row_top = Some(row_top);
                        }
                        assert!(ui.min_rect().width() <= width + 1.0);
                        assert!(
                            ui.min_rect().height() <= 100.0,
                            "progress spacing consumed the pane: {}",
                            ui.min_rect().height()
                        );
                    },
                );
            });
        });
    }
}

#[test]
fn expanded_settings_fit_narrow_forms_in_both_languages() {
    use good_tools_app::{
        config::StarRailSettings,
        gui::{
            manager_tab, scanner_tab, star_rail_scanner_tab,
            star_rail_state::StarRailState,
            state::{AppState, Lang},
        },
    };
    let ctx = egui::Context::default();
    theme::setup(&ctx);
    ctx.memory_mut(|memory| memory.set_everything_is_visible(true));
    let mut genshin = AppState::new();
    let mut settings = StarRailSettings::default();
    let mut hsr = StarRailState::new(String::new());
    for lang in [Lang::Zh, Lang::En] {
        genshin.lang = lang;
        for width in [250.0, 360.0] {
            for tool in 0..3 {
                let _ = ctx.run(Default::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        layout::region(
                            ui,
                            egui::Rect::from_min_size(
                                egui::pos2(20.0, 20.0),
                                egui::vec2(width, 2000.0),
                            ),
                            "expanded-settings",
                            |ui| {
                                match tool {
                                    0 => scanner_tab::show_settings(ui, &mut genshin, false),
                                    1 => manager_tab::show_settings(ui, &mut genshin, false),
                                    _ => star_rail_scanner_tab::show_settings(
                                        ui,
                                        lang,
                                        &mut settings,
                                        &mut hsr,
                                        false,
                                    ),
                                }
                                assert!(
                                    ui.min_rect().width() <= width + 1.0,
                                    "expanded settings exceeded {width}: {}",
                                    ui.min_rect().width()
                                );
                            },
                        );
                    });
                });
            }
        }
    }
}

#[test]
fn manager_file_controls_and_help_fit_with_long_windows_paths() {
    use good_tools_app::{
        config::StarRailSettings,
        gui::{star_rail_manager_tab, star_rail_state::StarRailState, state::Lang},
    };
    let ctx = egui::Context::default();
    theme::setup(&ctx);
    let mut settings = StarRailSettings::default();
    settings.manager_journal_path = format!(
        r"C:\Users\Player\{}\recovery.jsonl",
        "long-folder\\".repeat(12)
    );
    settings.manager_instructions_path = format!(
        r"C:\Users\Player\{}\instructions.json",
        "long-folder\\".repeat(12)
    );
    let mut state = StarRailState::new(String::new());
    for lang in [Lang::Zh, Lang::En] {
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                layout::region(
                    ui,
                    egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(360.0, 500.0)),
                    "manager-file-test",
                    |ui| {
                        star_rail_manager_tab::show_settings(
                            ui,
                            lang,
                            &mut settings,
                            &mut state,
                            false,
                        );
                        assert!(
                            ui.min_rect().width() <= 361.0,
                            "path controls expanded the settings pane: {}",
                            ui.min_rect().width()
                        );
                        assert!(
                            ui.min_rect().height() < 400.0,
                            "path controls introduced a tall empty row"
                        );
                    },
                );
            });
        });
    }
}

fn draw_split(
    ctx: &egui::Context,
    fraction: &mut f32,
    size: [f32; 2],
    events: Vec<Event>,
) -> [egui::Rect; 2] {
    let mut panes = [egui::Rect::NOTHING; 2];
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size.into())),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none())
                .show(ctx, |ui| {
                    panes = layout::split(
                        ui,
                        ui.max_rect(),
                        "sidebar",
                        fraction,
                        true,
                        good_tools_app::gui::shell::SIDEBAR_MIN_WIDTH,
                    );
                });
        },
    );
    panes
}

#[test]
fn splitter_drag_changes_ratio_and_resize_keeps_the_requested_ratio() {
    let ctx = egui::Context::default();
    theme::setup(&ctx);
    let mut fraction = 0.10;
    let panes = draw_split(&ctx, &mut fraction, [1040.0, 760.0], vec![]);
    assert_eq!(panes[0].right(), 145.0);
    let point = egui::pos2(148.0, 200.0);
    draw_split(
        &ctx,
        &mut fraction,
        [1040.0, 760.0],
        vec![
            Event::PointerMoved(point),
            Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    draw_split(
        &ctx,
        &mut fraction,
        [1040.0, 760.0],
        vec![Event::PointerMoved(egui::pos2(230.0, 200.0))],
    );
    draw_split(
        &ctx,
        &mut fraction,
        [1040.0, 760.0],
        vec![Event::PointerButton {
            pos: egui::pos2(230.0, 200.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!((fraction - 230.0 / 1040.0).abs() < 0.001);
    let requested = fraction;
    let panes = draw_split(&ctx, &mut fraction, [760.0, 500.0], vec![]);
    assert_eq!(fraction, requested);
    assert!((panes[0].right() + 3.0 - 760.0 * requested).abs() < 0.1);
    fraction = 0.10;
    let small = draw_split(&ctx, &mut fraction, [760.0, 500.0], vec![]);
    assert_eq!(
        fraction, 0.10,
        "minimum width must not overwrite the saved ratio"
    );
    assert_eq!(small[0].right(), 145.0);
}

#[test]
fn overflowing_pane_content_keeps_its_own_clip_rect() {
    let ctx = egui::Context::default();
    let _ = ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let expected =
                egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(200.0, 100.0));
            layout::region(ui, expected, "overflow", |ui| {
                ui.add(egui::Label::new("Very long unwrapped content ".repeat(100)).extend());
                ui.allocate_space(egui::vec2(1500.0, 900.0));
                assert_eq!(ui.clip_rect(), expected);
            });
        });
    });
}

#[test]
fn titlebar_stays_fixed_and_language_and_window_buttons_remain_clickable() {
    use good_tools_app::{
        config::Game,
        gui::{shell, state::Lang},
    };
    for (x, expected) in [
        (564.0, "language"),
        (638.0, "minimize"),
        (684.0, "maximize"),
        (730.0, "close"),
    ] {
        let ctx = egui::Context::default();
        theme::setup(&ctx);
        let logos = shell::Logos::load(&ctx);
        let mut lang = Lang::Zh;
        let mut game = Game::Genshin;
        let mut draw = |events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(760.0, 500.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    shell::titlebar(ctx, &mut lang, &mut game, true, &logos);
                    assert_eq!(ctx.available_rect().top(), shell::TITLEBAR_HEIGHT);
                },
            )
        };
        draw(vec![]);
        let pos = egui::pos2(x, 28.0);
        draw(vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]);
        let output = draw(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        drop(draw);
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        match expected {
            "language" => assert_eq!(lang, Lang::En),
            "close" => assert!(commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::Close))),
            "maximize" => assert!(commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::Maximized(true)))),
            "minimize" => assert!(commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::Minimized(true)))),
            _ => unreachable!(),
        }
    }
}

#[derive(Default)]
struct Store {
    saved: Option<ManagerJournal>,
    fail: bool,
    lease: bool,
}
impl ManagerJournalStore for Store {
    fn load(&mut self) -> Result<Option<ManagerJournal>, String> {
        Ok(self.saved.clone())
    }
    fn save(&mut self, journal: &ManagerJournal) -> Result<(), String> {
        if self.fail {
            return Err("disk-full".into());
        }
        self.saved = Some(journal.clone());
        Ok(())
    }
    fn holds_exclusive_apply_lease(&self) -> bool {
        self.lease
    }
}

fn fixture_journal() -> ManagerJournal {
    use hsr_scanner::manager::{
        build_manager_plan, ManagedGearObservation, ManagedState, ManagerInstructionsEnvelope,
    };
    let envelope = ManagerInstructionsEnvelope::parse_json(include_str!(
        "../../experimental/hsr/tests/fixtures/manager_instructions_v1.json"
    ))
    .unwrap();
    let plan = build_manager_plan(
        &envelope,
        &[ManagedGearObservation {
            matcher: envelope.instructions[0].matcher.clone(),
            state: ManagedState {
                lock: Some(false),
                discard: Some(false),
            },
            equipped: Some(false),
        }],
    )
    .unwrap();
    let entries = plan
        .entries
        .iter()
        .flat_map(|entry| {
            entry.changes.iter().cloned().map(|change| JournalEntry {
                instruction_id: entry.instruction_id.clone(),
                change,
                status: JournalStatus::Pending,
                toggle_attempts: 0,
                outcome_code: None,
            })
        })
        .collect();
    ManagerJournal {
        schema: hsr_scanner::manager::MANAGER_JOURNAL_SCHEMA.into(),
        schema_version: hsr_scanner::manager::MANAGER_JOURNAL_SCHEMA_VERSION,
        request_id: plan.request_id.clone(),
        idempotency_key: plan.idempotency_key.clone(),
        plan_digest: plan.digest.clone(),
        plan,
        entries,
    }
}

#[test]
fn manager_progress_waits_for_durable_verification_and_preserves_lease_and_stop() {
    let mut journal = fixture_journal();
    assert!(!journal.entries.is_empty());
    let mut initial = TaskProgress::default();
    plan_steps(&mut initial, &journal.plan);
    let progress = Arc::new(Mutex::new(initial));
    let status = Arc::new(Mutex::new(TaskStatus::Idle));
    let cancel = yas::cancel::CancelToken::new();
    let mut store = TrackedJournal {
        inner: Store {
            lease: true,
            ..Default::default()
        },
        progress: progress.clone(),
        status: status.clone(),
        cancel: cancel.clone(),
    };
    assert!(store.holds_exclusive_apply_lease());
    journal.entries[0].status = JournalStatus::MutationStarted;
    store.save(&journal).unwrap();
    assert_eq!(
        progress
            .lock()
            .unwrap()
            .steps
            .iter()
            .map(|s| s.completed)
            .sum::<usize>(),
        0
    );
    journal.entries[0].status = JournalStatus::Verified;
    store.inner.fail = true;
    assert_eq!(store.save(&journal).unwrap_err(), "disk-full");
    assert_eq!(
        progress
            .lock()
            .unwrap()
            .steps
            .iter()
            .map(|s| s.completed)
            .sum::<usize>(),
        0
    );
    store.inner.fail = false;
    store.save(&journal).unwrap();
    assert_eq!(
        progress
            .lock()
            .unwrap()
            .steps
            .iter()
            .map(|s| s.completed)
            .sum::<usize>(),
        1
    );
    cancel.cancel(yas::cancel::StopReason::UserAbort);
    *status.lock().unwrap() = TaskStatus::Running(UiText::new("正在停止", "Stopping"));
    journal.entries[0].status = JournalStatus::MutationStarted;
    store.save(&journal).unwrap();
    assert!(
        matches!(&*status.lock().unwrap(), TaskStatus::Running(text) if text.text(good_tools_app::gui::state::Lang::En) == "Stopping")
    );
    journal.entries[0].status = JournalStatus::NeedsReview;
    store.save(&journal).unwrap();
    assert!(progress
        .lock()
        .unwrap()
        .steps
        .iter()
        .any(|s| s.state == StepState::Interrupted));
    store.load().unwrap();
}
