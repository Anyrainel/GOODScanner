//! Isolated 1920×1080 layout for the in-game achievement screen.
//!
//! These numbers are **not** shared with backpack/character scanners. The
//! achievement UI is a left-hand category column plus a right-hand card list.

/// Paimon menu "成就" tile center at 1920×1080.
///
/// Calibrated from an Esc-menu screenshot: 4th column, first row of the icon
/// grid (trophy tile). ~34.5% from the left, ~34.7% from the top.
pub const PAIMON_ACHIEVEMENT_POS: (f64, f64) = (663.0, 375.0);

/// First category card ("天地万象") on the achievement overview grid.
/// Clicking this opens the left-sidebar list the rest of the scanner walks.
pub const OVERVIEW_FIRST_CATEGORY_POS: (f64, f64) = (217.0, 195.0);

/// Wait after Escape before the Paimon menu tiles are clickable (ms).
pub const PAIMON_MENU_DELAY: u32 = 800;

/// Left-side category column click X (center of a category row).
pub const CATEGORY_X: f64 = 214.0;

/// Y of the first visible category row center (天地万象).
pub const CATEGORY_FIRST_Y: f64 = 224.0;

/// Vertical pitch between left-list category rows.
pub const CATEGORY_STEP_Y: f64 = 104.0;

/// Fully visible left-list rows. Clicking the last one makes the game
/// auto-scroll, so the same slot then targets the next category.
pub const CATEGORY_VISIBLE: usize = 8;

/// Safety cap on category clicks (including left-list scrolls).
pub const MAX_CATEGORIES: usize = 128;

/// Cocogoat CaptureScanner: `MOUSEEVENTF_WHEEL` dwData=-120, `repeat=11`
/// per capture. One tick per OCR frame is what made the list crawl.
pub const LIST_WHEEL_TICKS: i32 = 11;

/// Consecutive captures with no new unique titles before treating the list as done.
/// Pixel-identical lists stop immediately; this fallback must be high enough
/// that two OCR misses in a row do not abandon a long category.
pub const IDLE_FRAMES_BEFORE_STOP: u32 = 6;

/// Safety cap so a flickering OCR key cannot scroll a category forever.
/// 天地万象 is hundreds of cards; 11 wheel ticks only move ~1.5 rows, so
/// a full pass can need ~800 frames. Pixel-identical title regions stop earlier.
pub const MAX_LIST_FRAMES: u32 = 1200;

/// Mean per-sample RGB delta below which two list captures are the same view.
pub const LIST_UNCHANGED_MEAN_DELTA: u64 = 6;

/// Left sidebar probe (fraction of window width) for the selected category row.
pub const CATEGORY_PROBE_X0: f32 = 0.04;
pub const CATEGORY_PROBE_X1: f32 = 0.20;
pub const CATEGORY_SELECTED_MIN_H: u32 = 50;
pub const CATEGORY_SELECTED_MAX_H: u32 = 160;

/// Drop a trailing split row shorter than this fraction of the median height.
/// Cocogoat used 2/3 of the mean row height.
pub const PARTIAL_ROW_HEIGHT_RATIO: f32 = 2.0 / 3.0;

/// Relative crops inside one achievement card (fractions of the row).
/// Layout: icon | title/subtitle on the left, status/date on the right.
pub const TITLE_FRAC: (f32, f32, f32, f32) = (0.10, 0.08, 0.48, 0.40);
pub const SUBTITLE_FRAC: (f32, f32, f32, f32) = (0.10, 0.50, 0.48, 0.42);
pub const STATUS_FRAC: (f32, f32, f32, f32) = (0.62, 0.08, 0.32, 0.40);
pub const DATE_FRAC: (f32, f32, f32, f32) = (0.62, 0.52, 0.32, 0.40);

/// Cream list-panel pixels. Card bodies sit around 230; sidebar/overworld are far darker.
pub const LIST_PANEL_GRAY: u8 = 200;

/// Hairline between two cream cards drops to ~206–213. Anything at or below this
/// in the cream column is treated as a row divider.
pub const ROW_DIVIDER_MAX: u8 = 216;

/// Consecutive dark rows allowed inside the cream panel (icons punch holes).
pub const LIST_RUN_GAP: u32 = 40;

/// Non-cream pixels allowed when merging the list's horizontal span (icon gaps).
pub const LIST_X_GAP: u32 = 80;

/// Right-hand band used to decide whether a row is "on the list panel".
pub const LIST_PROBE_X0: f32 = 0.40;
pub const LIST_PROBE_X1: f32 = 0.90;
pub const LIST_ROW_BRIGHT_RATIO: f32 = 0.55;

/// Minimum panel size vs the capture.
pub const LIST_MIN_WIDTH_RATIO: f32 = 0.50;
pub const LIST_MIN_HEIGHT_RATIO: f32 = 0.75;

/// Ignore divider hits closer than this to the panel top (inner-edge hairline).
pub const MIN_ROW_HEIGHT: u32 = 40;
