use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use hsr_scanner::scan_timing::ScanTimings;
use hsr_scanner::{TrailblazerGender, TrailblazerIdentity};
use serde::{Deserialize, Serialize};

pub const APPLICATION_CONFIG_SCHEMA_VERSION: u32 = 4;
pub const APPLICATION_CONFIG_FILE_REL: &str = "data/good_app_config.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Game {
    Genshin,
    StarRail,
}

impl Default for Game {
    fn default() -> Self {
        Self::Genshin
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolTab {
    Scanner,
    Manager,
    Capture,
    Credits,
}

impl ToolTab {
    pub fn default_for_build() -> Self {
        if cfg!(feature = "capture") {
            Self::Capture
        } else {
            Self::Scanner
        }
    }

    pub fn available_for_build(self) -> Self {
        if self == Self::Capture && !cfg!(feature = "capture") {
            Self::Scanner
        } else {
            self
        }
    }
}

impl Default for ToolTab {
    fn default() -> Self {
        Self::default_for_build()
    }
}

/// Navigation is stored independently for each game. Switching games changes
/// only `active_game`; returning to a game restores the tab the user left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameNavigation {
    #[serde(default)]
    pub active_game: Game,
    #[serde(default)]
    pub genshin_tab: ToolTab,
    #[serde(default)]
    pub star_rail_tab: ToolTab,
}

impl Default for GameNavigation {
    fn default() -> Self {
        Self {
            active_game: Game::Genshin,
            genshin_tab: ToolTab::default_for_build(),
            star_rail_tab: ToolTab::default_for_build(),
        }
    }
}

impl GameNavigation {
    pub fn active_tab(&self) -> ToolTab {
        match self.active_game {
            Game::Genshin => self.genshin_tab,
            Game::StarRail => self.star_rail_tab,
        }
        .available_for_build()
    }

    pub fn select_tab(&mut self, tab: ToolTab) {
        let tab = tab.available_for_build();
        match self.active_game {
            Game::Genshin => self.genshin_tab = tab,
            Game::StarRail => self.star_rail_tab = tab,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StarRailCaptureMethod {
    #[default]
    Auto,
    BitBlt,
    Wgc,
}

impl StarRailCaptureMethod {
    pub fn to_yas(self) -> Option<yas::capture::CaptureMethod> {
        match self {
            Self::Auto => None,
            Self::Wgc => Some(yas::capture::CaptureMethod::Wgc),
            Self::BitBlt => Some(yas::capture::CaptureMethod::BitBlt),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_manager_port() -> u16 {
    8765
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StarRailSettings {
    #[serde(default)]
    pub output_dir: String,
    #[serde(default = "default_true")]
    pub scan_characters: bool,
    #[serde(default = "default_true")]
    pub scan_light_cones: bool,
    #[serde(default = "default_true")]
    pub scan_relics_and_ornaments: bool,
    /// Zero scans the whole category. Reaching a cap gives partial coverage.
    #[serde(default)]
    pub max_characters: usize,
    #[serde(default)]
    pub max_light_cones: usize,
    #[serde(default)]
    pub max_gear: usize,
    #[serde(default)]
    pub capture_method: StarRailCaptureMethod,
    #[serde(default)]
    pub hdr_mode: bool,
    #[serde(default)]
    pub timings: ScanTimings,
    /// The Trailblazer's in-game nickname. Required for Character scans.
    #[serde(default)]
    pub trailblazer_name: String,
    #[serde(default)]
    pub trailblazer_gender: Option<TrailblazerGender>,
    #[serde(default)]
    pub dump_images: bool,
    #[serde(default)]
    pub manager_instructions_path: String,
    #[serde(default = "default_manager_port")]
    pub manager_port: u16,
    #[serde(default)]
    pub manager_journal_path: String,
    #[serde(default = "default_true")]
    pub capture_include_achievements: bool,
    #[serde(default = "default_true")]
    pub capture_include_characters: bool,
    #[serde(default = "default_true")]
    pub capture_include_light_cones: bool,
    #[serde(default = "default_true")]
    pub capture_include_relics: bool,
    #[serde(default)]
    pub capture_dump_packets: bool,
    #[serde(default)]
    pub capture_only_keep_latest_export: bool,
}

impl Default for StarRailSettings {
    fn default() -> Self {
        Self {
            output_dir: String::new(),
            scan_characters: true,
            scan_light_cones: true,
            scan_relics_and_ornaments: true,
            max_characters: 0,
            max_light_cones: 0,
            max_gear: 0,
            capture_method: StarRailCaptureMethod::default(),
            hdr_mode: false,
            timings: ScanTimings::default(),
            trailblazer_name: String::new(),
            trailblazer_gender: None,
            dump_images: false,
            manager_instructions_path: String::new(),
            manager_port: default_manager_port(),
            manager_journal_path: String::new(),
            capture_include_achievements: true,
            capture_include_characters: true,
            capture_include_light_cones: true,
            capture_include_relics: true,
            capture_dump_packets: false,
            capture_only_keep_latest_export: false,
        }
    }
}

impl StarRailSettings {
    /// GUI exports need no folder selection. Keep existing saved destinations working.
    pub fn export_directory(&self) -> PathBuf {
        if self.output_dir.trim().is_empty() {
            genshin_scanner::cli::exe_dir()
        } else {
            PathBuf::from(self.output_dir.trim())
        }
    }

    /// The normal HDR control restores the recommended capture policy.
    /// Advanced overrides can be selected afterward for troubleshooting.
    pub fn set_hdr_mode(&mut self, enabled: bool) {
        self.hdr_mode = enabled;
        self.capture_method = StarRailCaptureMethod::Auto;
    }

    /// `None` until both the nickname and the gender are filled in.
    pub fn trailblazer(&self) -> Option<TrailblazerIdentity> {
        let nickname = self.trailblazer_name.trim();
        if nickname.is_empty() {
            return None;
        }
        Some(TrailblazerIdentity {
            nickname: nickname.to_owned(),
            gender: self.trailblazer_gender?,
        })
    }

    pub fn missing_trailblazer(&self) -> bool {
        self.scan_characters && self.trailblazer().is_none()
    }

    fn populate_default_paths(&mut self, executable_dir: &Path) {
        if self.output_dir.trim().is_empty() {
            self.output_dir = executable_dir.display().to_string();
        }
        if self.manager_journal_path.trim().is_empty() {
            self.manager_journal_path = executable_dir
                .join("data")
                .join("hsr_manager_journal.jsonl")
                .display()
                .to_string();
        }
    }
}

fn current_schema_version() -> u32 {
    APPLICATION_CONFIG_SCHEMA_VERSION
}

fn migrate_star_rail_scan_settings(value: &mut serde_json::Value) -> Result<()> {
    use serde_json::Value;
    if !matches!(
        value.get("schemaVersion").and_then(Value::as_u64),
        Some(1..=3) | None
    ) {
        return Ok(());
    }
    let Some(settings) = value.get_mut("starRail").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    // v1/v2 and earlier local v3 files could save captureMethod="printWindow".
    // Retire that backend within the existing v3 migration from origin's v2;
    // Auto uses the saved hdrMode to select BitBlt or WGC.
    if settings.get("captureMethod").and_then(Value::as_str) == Some("printWindow") {
        settings.insert("captureMethod".to_owned(), Value::from("auto"));
    }
    if settings.contains_key("timings") {
        return Ok(());
    }
    // v1/v2 used navigationDelayMs (default 250) plus hidden per-path waits,
    // panelTimeoutMs, maxInventoryItems (an OCR rejection guard, default 4000),
    // maxCharacters (default 200), expectedCharacters, and nextCharacterKey.
    // Merge this transform into the existing v3 removal of archive imports.
    // Preserve total configured waits and intentional Character sample caps;
    // never reinterpret an inventory rejection guard as a sample size.
    let navigation = settings
        .remove("navigationDelayMs")
        .map(|value| {
            value
                .as_u64()
                .context("invalid Star Rail navigationDelayMs")
        })
        .transpose()?
        .unwrap_or(250);
    let timeout = settings
        .remove("panelTimeoutMs")
        .map(|value| value.as_u64().context("invalid Star Rail panelTimeoutMs"))
        .transpose()?
        .unwrap_or(900);
    let timings = ScanTimings {
        menu_open_ms: navigation.saturating_add(1_500),
        menu_close_ms: navigation.saturating_add(1_000),
        inventory_tab_ms: navigation.saturating_add(1_500),
        panel_switch_ms: navigation.saturating_add(500),
        traces_open_ms: navigation.saturating_add(2_000),
        panel_timeout_ms: timeout,
        status_toggle_ms: navigation,
        ..ScanTimings::default()
    };
    settings.insert("timings".to_owned(), serde_json::to_value(timings)?);
    if settings.get("maxCharacters").and_then(Value::as_u64) == Some(200) {
        settings.insert("maxCharacters".to_owned(), Value::from(0));
    }
    for key in [
        "maxInventoryItems",
        "expectedCharacters",
        "nextCharacterKey",
    ] {
        settings.remove(key);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUiConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub navigation: GameNavigation,
    #[serde(default)]
    pub star_rail: StarRailSettings,
}

impl Default for ApplicationUiConfig {
    fn default() -> Self {
        Self {
            schema_version: APPLICATION_CONFIG_SCHEMA_VERSION,
            navigation: GameNavigation::default(),
            star_rail: StarRailSettings::default(),
        }
    }
}

impl ApplicationUiConfig {
    fn validate_version(&self) -> Result<()> {
        if self.schema_version != APPLICATION_CONFIG_SCHEMA_VERSION {
            bail!(
                "unsupported application config schemaVersion={}; expected={APPLICATION_CONFIG_SCHEMA_VERSION}",
                self.schema_version
            );
        }
        Ok(())
    }
}

/// Independent persistence boundary for cross-game navigation and Star Rail
/// settings. Genshin's long-standing `good_config.json` remains owned by
/// `GoodUserConfig`, including when its CLI rewrites that file.
pub struct ApplicationConfigStore {
    path: PathBuf,
    pub config: ApplicationUiConfig,
    snapshot: String,
    dirty_since: Option<Instant>,
    /// A malformed or newer-version file is never silently replaced. The
    /// fallback remains read-only for this whole session; users can preserve
    /// or manually rename the original before starting a fresh config.
    writes_blocked: bool,
}

impl ApplicationConfigStore {
    pub fn for_running_executable() -> (Self, Option<anyhow::Error>) {
        let executable_dir = genshin_scanner::cli::exe_dir();
        Self::for_executable_dir(&executable_dir)
    }

    pub fn for_executable_dir(executable_dir: &Path) -> (Self, Option<anyhow::Error>) {
        let path = executable_dir.join(APPLICATION_CONFIG_FILE_REL);
        let (mut store, warning) = Self::load_or_default(path);
        store
            .config
            .star_rail
            .populate_default_paths(executable_dir);
        // Path defaults are startup-derived rather than a user's decision.
        // Treat them as the in-memory baseline so they cannot overwrite an
        // unreadable or future-version file on debounce or shutdown.
        store.snapshot = serde_json::to_string(&store.config).unwrap_or_default();
        store.dirty_since = None;
        (store, warning)
    }

    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let mut config = if path.exists() {
            let json = fs::read_to_string(&path)
                .with_context(|| format!("could not read {}", path.display()))?;
            let mut value: serde_json::Value = serde_json::from_str(&json)
                .with_context(|| format!("could not parse {}", path.display()))?;
            migrate_star_rail_scan_settings(&mut value)?;
            serde_json::from_value::<ApplicationUiConfig>(value)
                .with_context(|| format!("could not parse {}", path.display()))?
        } else {
            ApplicationUiConfig::default()
        };
        // v1 had starRail.referenceBundle: an optional developer-owned folder
        // path. v1/v2 had starRail.offlineImportPath: the JSON converter's input
        // path. Serde discards those obsolete string keys while retaining all
        // scan, capture, manager, navigation and output settings.
        // v1/v2 and earlier local v3 files stored a concrete captureMethod
        // (default bitBlt) and no HDR
        // setting. Preserve supported choices as advanced overrides; an omitted
        // method now uses Auto, which still selects BitBlt with HDR disabled.
        // v3 added HDR. v4 adds the loopback manager port; the serde default
        // preserves every existing preference and supplies port 8765.
        if matches!(config.schema_version, 1 | 2 | 3) {
            config.schema_version = APPLICATION_CONFIG_SCHEMA_VERSION;
        }
        config.validate_version()?;
        let snapshot = serde_json::to_string(&config)?;
        Ok(Self {
            path,
            config,
            snapshot,
            dirty_since: None,
            writes_blocked: false,
        })
    }

    pub fn load_or_default(path: impl Into<PathBuf>) -> (Self, Option<anyhow::Error>) {
        let path = path.into();
        match Self::load(path.clone()) {
            Ok(store) => (store, None),
            Err(error) => {
                let config = ApplicationUiConfig::default();
                let snapshot = serde_json::to_string(&config).unwrap_or_default();
                (
                    Self {
                        path,
                        config,
                        snapshot,
                        dirty_since: None,
                        writes_blocked: true,
                    },
                    Some(error),
                )
            },
        }
    }

    pub fn auto_save_tick(&mut self) -> Result<()> {
        if self.writes_blocked {
            self.dirty_since = None;
            return Ok(());
        }
        let current = serde_json::to_string(&self.config)?;
        if current != self.snapshot {
            self.snapshot = current;
            self.dirty_since = Some(Instant::now());
        }
        if self
            .dirty_since
            .is_some_and(|since| since.elapsed() >= Duration::from_millis(300))
        {
            self.persist_now()?;
        }
        Ok(())
    }

    pub fn persist_now(&mut self) -> Result<()> {
        if self.writes_blocked {
            self.dirty_since = None;
            return Ok(());
        }
        let current = serde_json::to_string(&self.config)?;
        if current != self.snapshot {
            // This covers a last UI edit followed by immediate window close,
            // before another update frame had a chance to run auto_save_tick.
            self.snapshot = current;
        }
        let result = (|| {
            self.config.validate_version()?;
            let json = serde_json::to_string_pretty(&self.config)?;
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("could not create {}", parent.display()))?;
            }
            fs::write(&self.path, format!("{json}\n"))
                .with_context(|| format!("could not write {}", self.path.display()))?;
            Ok::<_, anyhow::Error>(())
        })();
        // A failed attempt is reported once. A later settings change starts a
        // new debounce window and retries; shutdown also calls persist_now.
        self.dirty_since = None;
        result?;
        self.snapshot = serde_json::to_string(&self.config)?;
        Ok(())
    }

    pub fn save_pending(&self) -> bool {
        self.dirty_since.is_some()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
