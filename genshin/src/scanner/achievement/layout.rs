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

/// Physical mouse-wheel notches to advance one achievement card.
///
/// Hand-counted: 8 distinct notches just pass one entry. Cocogoat's 11-tick
/// burst was sent with no delay, so Windows coalesced it and the list jumped
/// unpredictably (sometimes ~2 cards, sometimes a few pixels).
pub const LIST_WHEEL_TICKS: i32 = 8;

/// One Windows wheel detent. `MOUSEEVENTF_WHEEL` dwData is this value, not 1.
#[allow(dead_code)]
pub const LIST_WHEEL_DELTA: i32 = 120;

/// Pause between detents so Genshin registers each `WM_MOUSEWHEEL`.
/// 25ms with a delay only every 5 ticks still coalesced (~2 cards/frame).
pub const LIST_TICK_DELAY_MS: u32 = 35;

/// After moving onto the scrollbar, wait for any hovered card to un-enlarge.
pub const LIST_HOVER_SETTLE_MS: u32 = 40;

/// Center of the right-hand list scrollbar at 1920×1080.
///
/// Hovering a card enlarges it and wrecks row geometry. Park here (move, do
/// not click — a click on the track jumps the list) and wheel from this gutter.
pub const LIST_SCROLLBAR_POS: (f64, f64) = (1838.0, 560.0);

/// "成就" header. A click here focuses the game without hovering a card.
pub const ACHIEVEMENT_HEADER_POS: (f64, f64) = (95.0, 42.0);

/// Pixels to the right of [`detect_list_rect`]'s cream panel to sit on the
/// thin scrollbar track. The detector insets the panel, so the track is
/// outside that rect.
pub const LIST_SCROLLBAR_INSET: u32 = 16;

/// Consecutive captures with no new unique titles before treating the list as done.
/// Pixel-identical lists stop immediately; this fallback must be high enough
/// that two OCR misses in a row do not abandon a long category.
pub const IDLE_FRAMES_BEFORE_STOP: u32 = 6;

/// Safety cap so a flickering OCR key cannot scroll a category forever.
/// 天地万象 is hundreds of cards; 8 delayed notches ≈ 1 card, so a full pass
/// is a few hundred frames. Pixel-identical title regions stop earlier.
pub const MAX_LIST_FRAMES: u32 = 1200;

/// Mean per-sample RGB delta below which two list captures are the same view.
pub const LIST_UNCHANGED_MEAN_DELTA: u64 = 6;

/// Left sidebar probe (fraction of window width) for the selected category row.
pub const CATEGORY_PROBE_X0: f32 = 0.04;
/// Right edge of the sidebar cream, just left of the achievement list.
/// Measured on a 1920-wide frame: the selected row runs to x≈668 (0.35)
/// and the list panel starts at x≈712 (0.37). 0.24 clipped 「其一/其二」.
pub const CATEGORY_PROBE_X1: f32 = 0.36;
/// Pixels from the cream row's left edge to the category name.
pub const CATEGORY_NAME_SHIFT_X: u32 = 28;
pub const CATEGORY_SELECTED_MIN_H: u32 = 50;
pub const CATEGORY_SELECTED_MAX_H: u32 = 160;

/// Drop a leading split row shorter than this fraction of the median height.
pub const PARTIAL_ROW_HEIGHT_RATIO: f32 = 2.0 / 3.0;

/// Keep a last card if it is still tall enough for the title band (~100px).
/// 0.84 dropped c00 list_000's last title (`风带来了故事的种子`).
pub const TRAILING_PARTIAL_RATIO: f32 = 0.72;

/// Empty cream below the last card on a short category is taller than a card.
pub const TRAILING_MAX_RATIO: f32 = 1.45;

/// Pixel bands on a ~124px card. `y`/`h` are pixels from the card top so a
/// truncated last row does not scale the window into empty cream.
///
/// Measured on c00 list_000 card 1: title ink is about 22px tall and starts
/// near y=38. A 24px window sat on the baseline and shaved every glyph.
/// Subtitle ink starts ~15px below the title. 30px clears the baseline;
/// 34px reached the next line on short titles and the model read both.
/// 达成/date sit in x≈0.85–0.99, not on the primogem.
pub const TITLE_BAND: (f32, u32, f32, u32) = (0.078, 34, 0.50, 30);
/// Full cards are this tall. Title ink sits 36px below the card top on a
/// complete row, so a 141px split (banner gap glued on) and a 100px tail
/// both become the same window.
pub const CARD_HEIGHT: u32 = 125;
pub const SUBTITLE_BAND: (f32, u32, f32, u32) = (0.078, 64, 0.50, 52);
/// 达成 / n/m / date in one right-hand strip (they are one UI column).
pub const STATUS_BAND: (f32, u32, f32, u32) = (0.855, 12, 0.140, 108);

/// Grow a short cream hit (often just the "100 %" line) up to a full
/// category row so the name is in the OCR crop.
pub const CATEGORY_ROW_H: u32 = 100;

/// Cream list-panel pixels. Card bodies sit around 230; sidebar/overworld are far darker.
pub const LIST_PANEL_GRAY: u8 = 200;

/// Hairline between two cream cards drops to ~206–213. Anything at or below this
/// in the cream column is treated as a row divider.
pub const ROW_DIVIDER_MAX: u8 = 216;

/// A divider row must be this dark across the title column, not a single
/// sample. Subtitle glyphs hit ~0.25–0.70; real hairlines are ~1.0.
pub const LIST_DIVIDER_DARK_RATIO: f32 = 0.80;

/// Merge adjacent split fragments shorter than a real card (~124px).
pub const MIN_CARD_HEIGHT: u32 = 100;

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
