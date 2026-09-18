//! Isolated 1920×1080 layout for the in-game achievement screen.
//!
//! These numbers are **not** shared with backpack/character scanners. The
//! achievement UI is a left-hand category column plus a right-hand card list.

/// Fallback list panel when auto-detect cannot find a large bright rectangle.
/// `(x, y, w, h)` at 1920×1080.
pub const LIST_RECT: (f64, f64, f64, f64) = (416.0, 118.0, 1064.0, 860.0);

/// Category title strip at the top of the right-hand list.
pub const LIST_TITLE_RECT: (f64, f64, f64, f64) = (430.0, 78.0, 420.0, 36.0);

/// Left-side category column click X (center of a category row).
pub const CATEGORY_X: f64 = 214.0;

/// Y of the first visible category row center.
pub const CATEGORY_FIRST_Y: f64 = 168.0;

/// Vertical pitch between category rows.
pub const CATEGORY_STEP_Y: f64 = 76.0;

/// How many category rows are visible without scrolling the left list.
pub const CATEGORY_VISIBLE: usize = 10;

/// Safety cap on category clicks (including left-list scrolls).
pub const MAX_CATEGORIES: usize = 64;

/// Consecutive captures with no new unique rows before treating the list as done.
/// Cocogoat used 5; one extra covers a slow last-row animation.
pub const IDLE_FRAMES_BEFORE_STOP: u32 = 6;

/// Drop a trailing split row shorter than this fraction of the median height.
/// Cocogoat used 2/3 of the mean row height.
pub const PARTIAL_ROW_HEIGHT_RATIO: f32 = 2.0 / 3.0;

/// Relative crops inside one achievement card (fractions of the row).
/// Layout: icon | title/subtitle on the left, status/date on the right.
pub const TITLE_FRAC: (f32, f32, f32, f32) = (0.10, 0.08, 0.48, 0.40);
pub const SUBTITLE_FRAC: (f32, f32, f32, f32) = (0.10, 0.50, 0.48, 0.42);
pub const STATUS_FRAC: (f32, f32, f32, f32) = (0.62, 0.08, 0.32, 0.40);
pub const DATE_FRAC: (f32, f32, f32, f32) = (0.62, 0.52, 0.32, 0.40);

/// Gray threshold used by cocogoat `cvGetRect` (binary at 200).
pub const LIST_PANEL_GRAY: u8 = 200;

/// Gray threshold used by cocogoat `cvSplitImage` (binary at 223).
pub const ROW_SEPARATOR_GRAY: u8 = 223;

/// Minimum panel size vs the capture, matching cocogoat's contour filters.
pub const LIST_MIN_WIDTH_RATIO: f32 = 0.50;
pub const LIST_MIN_HEIGHT_RATIO: f32 = 0.75;
