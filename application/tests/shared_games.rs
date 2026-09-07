use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use good_tools_app::{
    config::{ApplicationConfigStore, Game, GameNavigation, ToolTab, APPLICATION_CONFIG_FILE_REL},
    gui::{game_switcher, state::Lang},
};
use hsr_scanner::manager::{
    build_manager_plan, JournalEntry, JournalStatus, ManagedGearObservation, ManagedState,
    ManagerInstructionsEnvelope, ManagerJournal, ManagerJournalStore, ManagerPlan,
    MANAGER_JOURNAL_SCHEMA, MANAGER_JOURNAL_SCHEMA_VERSION,
};
const MANAGER_INSTRUCTIONS: &str =
    include_str!("../../experimental/hsr/tests/fixtures/manager_instructions_v1.json");

#[derive(Default)]
struct MemoryJournalStore {
    journal: Option<ManagerJournal>,
}

impl ManagerJournalStore for MemoryJournalStore {
    fn load(&mut self) -> Result<Option<ManagerJournal>, String> {
        Ok(self.journal.clone())
    }

    fn save(&mut self, journal: &ManagerJournal) -> Result<(), String> {
        self.journal = Some(journal.clone());
        Ok(())
    }

    fn holds_exclusive_apply_lease(&self) -> bool {
        false
    }
}

fn manager_envelope() -> ManagerInstructionsEnvelope {
    ManagerInstructionsEnvelope::parse_json(MANAGER_INSTRUCTIONS)
        .expect("manager instruction fixture should be valid")
}

fn manager_plan(envelope: &ManagerInstructionsEnvelope) -> ManagerPlan {
    build_manager_plan(
        envelope,
        &[ManagedGearObservation {
            matcher: envelope.instructions[0].matcher.clone(),
            state: ManagedState {
                lock: Some(false),
                discard: Some(false),
            },
            equipped: Some(false),
        }],
    )
    .expect("matching observation should produce a valid manager plan")
}

fn manager_journal(plan: &ManagerPlan, status: JournalStatus) -> ManagerJournal {
    let (toggle_attempts, outcome_code) = match status {
        JournalStatus::Pending => (0, None),
        JournalStatus::MutationStarted => (1, None),
        JournalStatus::Verified => (1, Some("postverified".to_owned())),
        JournalStatus::NeedsReview => (1, Some("interrupted_mutation_ambiguous".to_owned())),
    };
    ManagerJournal {
        schema: MANAGER_JOURNAL_SCHEMA.to_owned(),
        schema_version: MANAGER_JOURNAL_SCHEMA_VERSION,
        request_id: plan.request_id.clone(),
        idempotency_key: plan.idempotency_key.clone(),
        plan_digest: plan.digest.clone(),
        plan: plan.clone(),
        entries: vec![JournalEntry {
            instruction_id: plan.entries[0].instruction_id.clone(),
            change: plan.entries[0].changes[0].clone(),
            status,
            toggle_attempts,
            outcome_code,
        }],
    }
}

fn temp_root(test_name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "goodscanner-{test_name}-{}-{nonce}",
        std::process::id()
    ))
}

fn remove_test_tree(root: &PathBuf) {
    if let Err(error) = fs::remove_dir_all(root) {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::NotFound,
            "could not remove test directory {}: {error}",
            root.display()
        );
    }
}

#[test]
fn switcher_gives_both_games_equal_resting_width() {
    for width in [600.0, 920.0] {
        eframe::egui::__run_test_ui(|ui| {
            ui.set_width(width);
            let available = ui.available_width();
            let gap = ui.spacing().item_spacing.x;
            let expected = (available - gap) / 2.0;
            let mut active_game = Game::Genshin;

            let layout = game_switcher::show(ui, Lang::En, &mut active_game, true);

            assert_eq!(active_game, Game::Genshin);
            assert!((layout.genshin_width - layout.star_rail_width).abs() <= 0.5);
            assert!((layout.genshin_width - expected).abs() <= 0.5);
            assert!((layout.star_rail_width - expected).abs() <= 0.5);
        });
    }
}

#[test]
fn default_star_rail_settings_use_automatic_hosted_reference_without_a_folder() {
    let root = temp_root("embedded-reference-default");
    let (_store, warning) = ApplicationConfigStore::for_executable_dir(&root);
    assert!(warning.is_none());
    assert!(
        !root.join("data").join("hsr_reference").exists(),
        "startup must not manufacture or require an external reference folder"
    );

    assert_eq!(
        hsr_scanner::data_cache::DATA_CACHE_URL,
        "https://hsr.ggartifact.com/good/hsr_data_cache.json"
    );
    assert_eq!(hsr_scanner::data_cache::DATA_CACHE_DIRECTORY, "data/hsr");

    remove_test_tree(&root);
}

#[test]
fn obsolete_custom_reference_path_is_ignored_when_loading_old_settings() {
    // Version 1 serialized an optional developer referenceBundle path. Its
    // removal discards only that setting; player scan preferences survive.
    let root = temp_root("v1-reference-removal");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("config.json");
    fs::write(&path,
        r#"{"schemaVersion":1,"starRail":{"referenceBundle":"C:\\missing-data","scanCharacters":false,"outputDir":"D:\\exports"}}"#
    ).unwrap();
    let store = ApplicationConfigStore::load(&path).unwrap();
    assert_eq!(store.config.schema_version, 2);
    let settings = &store.config.star_rail;
    assert!(!settings.scan_characters);
    assert!(settings.capture_include_characters);
    assert!(settings.capture_include_light_cones);
    assert!(settings.capture_include_relics);
    assert!(settings.capture_include_achievements);
    assert_eq!(settings.output_dir, r"D:\exports");
    assert!(!serde_json::to_string(&settings)
        .unwrap()
        .contains("referenceBundle"));
    remove_test_tree(&root);
}

#[test]
fn navigation_defaults_to_genshin_and_restores_each_games_tab() {
    let mut navigation = GameNavigation::default();
    assert_eq!(navigation.active_game, Game::Genshin);
    assert_eq!(navigation.active_tab(), ToolTab::default_for_build());

    navigation.select_tab(ToolTab::Manager);
    navigation.active_game = Game::StarRail;
    assert_eq!(navigation.active_tab(), ToolTab::default_for_build());
    navigation.select_tab(ToolTab::Credits);

    navigation.active_game = Game::Genshin;
    assert_eq!(navigation.active_tab(), ToolTab::Manager);
    navigation.active_game = Game::StarRail;
    assert_eq!(navigation.active_tab(), ToolTab::Credits);
}

#[test]
fn unavailable_capture_tab_routes_to_scanner_without_changing_other_game() {
    let mut navigation = GameNavigation {
        active_game: Game::Genshin,
        genshin_tab: ToolTab::Manager,
        star_rail_tab: ToolTab::Capture,
    };

    navigation.active_game = Game::StarRail;
    if cfg!(feature = "capture") {
        assert_eq!(navigation.active_tab(), ToolTab::Capture);
    } else {
        assert_eq!(navigation.active_tab(), ToolTab::Scanner);
    }
    assert_eq!(navigation.genshin_tab, ToolTab::Manager);
    assert_eq!(
        navigation.star_rail_tab,
        ToolTab::Capture,
        "runtime fallback must not erase GOODCapture's saved tab"
    );
}

#[test]
fn star_rail_settings_and_per_game_navigation_round_trip_separately() {
    let root = temp_root("shared-config-round-trip");
    let app_config_path = root.join(APPLICATION_CONFIG_FILE_REL);
    let genshin_config_path = root.join("data").join("good_config.json");
    fs::create_dir_all(
        genshin_config_path
            .parent()
            .expect("Genshin config path must have a parent"),
    )
    .expect("test config directory should be creatable");
    let genshin_sentinel = "{\"existingGenshinConfig\":true}\n";
    fs::write(&genshin_config_path, genshin_sentinel)
        .expect("Genshin sentinel config should be writable");

    let mut store =
        ApplicationConfigStore::load(&app_config_path).expect("default app config should load");
    store.config.navigation.active_game = Game::StarRail;
    store.config.navigation.genshin_tab = ToolTab::Manager;
    store.config.navigation.star_rail_tab = ToolTab::Capture;
    store.config.star_rail.output_dir = "D:\\exports\\star-rail".to_owned();
    store.config.star_rail.scan_characters = false;
    store.config.star_rail.max_inventory_items = 789;
    store
        .persist_now()
        .expect("separate application config should save");

    assert_eq!(
        fs::read_to_string(&genshin_config_path).expect("Genshin sentinel should remain readable"),
        genshin_sentinel,
        "saving Star Rail settings must not rewrite GoodUserConfig"
    );

    let reloaded =
        ApplicationConfigStore::load(&app_config_path).expect("saved app config should reload");
    assert_eq!(reloaded.config.navigation.active_game, Game::StarRail);
    assert_eq!(reloaded.config.navigation.genshin_tab, ToolTab::Manager);
    assert_eq!(reloaded.config.navigation.star_rail_tab, ToolTab::Capture);
    assert_eq!(
        reloaded.config.star_rail.output_dir,
        "D:\\exports\\star-rail"
    );
    assert!(!reloaded.config.star_rail.scan_characters);
    assert_eq!(reloaded.config.star_rail.max_inventory_items, 789);

    remove_test_tree(&root);
}

#[test]
fn unsupported_config_is_not_overwritten_by_startup_or_shutdown() {
    let root = temp_root("future-config-preserved");
    let app_config_path = root.join(APPLICATION_CONFIG_FILE_REL);
    fs::create_dir_all(
        app_config_path
            .parent()
            .expect("application config path must have a parent"),
    )
    .expect("test config directory should be creatable");
    let future_config = concat!(
        "{\n",
        "  \"schemaVersion\": 999,\n",
        "  \"navigation\": {\n",
        "    \"activeGame\": \"starRail\",\n",
        "    \"genshinTab\": \"manager\",\n",
        "    \"starRailTab\": \"credits\"\n",
        "  },\n",
        "  \"starRail\": {}\n",
        "}\n",
    );
    fs::write(&app_config_path, future_config).expect("future config fixture should be writable");

    let (mut store, warning) = ApplicationConfigStore::for_executable_dir(&root);
    assert!(warning.is_some(), "future schema must surface a warning");
    assert_eq!(
        store.config.star_rail.output_dir,
        root.display().to_string()
    );
    assert!(!store.save_pending());

    // Ordinary navigation/settings interaction is not permission to destroy
    // a config written by a newer version of the application.
    store.config.navigation.active_game = Game::StarRail;
    store.config.star_rail.max_inventory_items = 456;
    store
        .auto_save_tick()
        .expect("future-schema fallback should remain read-only");
    store
        .persist_now()
        .expect("shutdown after UI edits should leave the future config untouched");
    assert_eq!(
        fs::read_to_string(&app_config_path).expect("future config should remain readable"),
        future_config
    );

    remove_test_tree(&root);
}

#[test]
fn manager_adapter_surfaces_original_interrupted_plan_as_recovery_preview() {
    let envelope = manager_envelope();
    let plan = manager_plan(&envelope);
    let mut store = MemoryJournalStore {
        journal: Some(manager_journal(&plan, JournalStatus::MutationStarted)),
    };

    let preview = good_tools_app::gui::star_rail_worker::load_recovery_preview(
        &envelope,
        &mut store,
        "settings-v1".to_owned(),
    )
    .expect("valid interrupted journal should load")
    .expect("interrupted journal should produce a recovery preview");

    assert!(preview.recovered);
    assert_eq!(preview.plan, plan);
    assert_eq!(preview.settings_identity, "settings-v1");
    assert!(preview.exact_json.contains(&preview.plan.digest));
}

#[test]
fn completed_journal_is_archived_before_a_distinct_plan_uses_the_base_path() {
    let root = temp_root("completed-manager-journal");
    fs::create_dir_all(&root).expect("manager journal directory should be creatable");
    let journal_path = root.join("hsr_manager_journal.jsonl");
    let first_envelope = manager_envelope();
    let first_plan = manager_plan(&first_envelope);
    let pending_journal = manager_journal(&first_plan, JournalStatus::Pending);
    let started_journal = manager_journal(&first_plan, JournalStatus::MutationStarted);
    let first_journal = manager_journal(&first_plan, JournalStatus::Verified);
    fs::write(
        &journal_path,
        format!(
            "{}\n{}\n{}\n",
            serde_json::to_string(&pending_journal)
                .expect("pending manager journal should serialize"),
            serde_json::to_string(&started_journal)
                .expect("started manager journal should serialize"),
            serde_json::to_string(&first_journal)
                .expect("verified manager journal should serialize")
        ),
    )
    .expect("verified manager journal should be writable");

    let mut first_store = hsr_scanner::manager::AppendOnlyJsonJournalStore::new(&journal_path);
    let recovered = good_tools_app::gui::star_rail_worker::load_recovery_preview(
        &first_envelope,
        &mut first_store,
        "settings-v1".to_owned(),
    )
    .expect("verified journal should still be safely recoverable")
    .expect("verified journal should retain its original plan");
    assert!(recovered.recovered);

    let archived =
        good_tools_app::gui::star_rail_worker::archive_completed_manager_journal(&journal_path)
            .expect("completed journal should archive")
            .expect("completed journal file should exist");
    assert!(!journal_path.exists());
    assert!(archived.exists());

    let mut second_envelope = first_envelope;
    second_envelope.request_id = "second-request".to_owned();
    second_envelope.instructions[0].id = "hsr-manager-0002-lock".to_owned();
    second_envelope.idempotency_key = second_envelope
        .expected_idempotency_key()
        .expect("second instruction set should have a deterministic key");
    let mut second_store = hsr_scanner::manager::AppendOnlyJsonJournalStore::new(&journal_path);
    let second_preview = good_tools_app::gui::star_rail_worker::load_recovery_preview(
        &second_envelope,
        &mut second_store,
        "settings-v2".to_owned(),
    )
    .expect("a distinct plan should not be blocked by the archived journal");
    assert!(second_preview.is_none());

    remove_test_tree(&root);
}

#[cfg(feature = "capture")]
#[test]
fn stop_before_worker_start_never_opens_capture_boundary() {
    assert!(
        good_tools_app::gui::star_rail_capture_tab::stop_before_worker_start_suppresses_start_for_test(),
        "a Stop request during reference loading must suppress the worker's later Start command"
    );
}

#[cfg(feature = "capture")]
#[test]
fn completed_capture_advances_and_exports_while_its_tab_is_inactive() {
    let root = temp_root("inactive-capture-completion");
    let output_dir = root.join("exports");
    let references = hsr_scanner::load_embedded_gilore_reference().unwrap();
    let achievement_id = references.achievement_ids().next().unwrap();

    let mut capture = good_tools_app::gui::star_rail_capture_tab::StarRailCaptureState::new(
        output_dir.display().to_string(),
    );
    let mut inventory: hsr_scanner::ObservationSnapshot = serde_json::from_str(include_str!(
        "../../experimental/hsr/tests/fixtures/observations.json"
    ))
    .unwrap();
    inventory.evidence.kind = hsr_scanner::EvidenceKind::PacketCapture;
    for gear in &mut inventory.gear {
        gear.main_stat_value = references
            .relic_main_stat_value_for_piece(
                references.gear(gear.piece_id).unwrap(),
                &gear.main_stat_key,
                gear.level,
            )
            .unwrap();
    }
    capture.inject_completed_for_test(references, vec![achievement_id], inventory);

    // Deliberately never render the Capture tab. GuiApp's unconditional tick
    // must still advance completion, stop cleanup, and export delivery.
    let exported_path = (0..200).find_map(|_| {
        capture.tick();
        let path = capture.completed_export_path_for_test().map(PathBuf::from);
        if path.is_none() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        path
    });
    let exported_path = exported_path.expect("off-screen capture should finish its export");
    let export: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&exported_path).expect("achievement export should be readable"),
    )
    .expect("achievement export should be valid JSON");

    assert_eq!(export["schema"], "goodscanner.hsr");
    assert_eq!(export["schemaVersion"], 3);
    assert_eq!(export["source"]["coverage"]["characters"], "complete");
    assert_eq!(export["source"]["coverage"]["lightCones"], "complete");
    assert_eq!(export["source"]["coverage"]["relics"], "complete");
    assert!(!export["characters"].as_array().unwrap().is_empty());
    assert!(!export["lightCones"].as_array().unwrap().is_empty());
    assert!(!export["relics"].as_array().unwrap().is_empty());
    assert_eq!(export["achievements"]["coverage"], "complete");
    assert_eq!(
        export["achievements"]["entries"][0]["achievementId"],
        achievement_id
    );
    assert_eq!(export["achievements"]["entries"][0]["status"], "completed");

    let stamp = exported_path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .strip_prefix("star_rail_export_")
        .unwrap();
    let common: serde_json::Value = serde_json::from_slice(
        &fs::read(output_dir.join(format!("star_rail_fribbels_{stamp}"))).unwrap(),
    )
    .unwrap();
    assert_eq!(common["source"], "HSR-Scanner");
    assert_eq!(common["version"], 4);
    assert_eq!(common["generator"]["name"], "GOODScanner");
    assert!(common["metadata"]["uid"].is_null());
    assert_eq!(
        common["relics"].as_array().unwrap().len(),
        export["relics"].as_array().unwrap().len()
            + export["planarOrnaments"].as_array().unwrap().len()
    );
    let stardb: serde_json::Value = serde_json::from_slice(
        &fs::read(output_dir.join(format!("star_rail_achievements_{stamp}"))).unwrap(),
    )
    .unwrap();
    assert_eq!(
        stardb,
        serde_json::json!({"hsr_achievements": [achievement_id]})
    );
    if let Ok(path) = std::env::var("HSR_INTEROP_FIXTURE") {
        fs::write(path, serde_json::to_vec_pretty(&common).unwrap()).unwrap();
    }
    remove_test_tree(&root);
}

#[test]
fn capture_selection_defaults_and_saved_choices_are_independent_of_ocr() {
    // Older schema-2 settings always captured inventory and had only the achievement toggle.
    let mut settings: good_tools_app::config::StarRailSettings =
        serde_json::from_str(r#"{"captureIncludeAchievements":false,"scanCharacters":false}"#)
            .unwrap();
    assert!(
        settings.capture_include_characters
            && settings.capture_include_light_cones
            && settings.capture_include_relics
    );
    assert!(!settings.capture_include_achievements);
    assert!(!settings.capture_dump_packets && !settings.capture_only_keep_latest_export);
    settings.capture_dump_packets = true;
    settings.capture_only_keep_latest_export = true;
    settings.capture_include_characters = false;
    settings.capture_include_relics = false;
    let restored: good_tools_app::config::StarRailSettings =
        serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    assert!(
        !restored.capture_include_characters
            && restored.capture_include_light_cones
            && !restored.capture_include_relics
            && !restored.capture_include_achievements
    );
    assert!(restored.scan_light_cones && restored.scan_relics_and_ornaments);
    assert!(restored.capture_dump_packets && restored.capture_only_keep_latest_export);
}
