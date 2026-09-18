/// Wait after a list scroll tick, before the next capture (ms).
/// Cocogoat measured ~80–160 ms of wheel latency; stay on the low side.
pub const DEFAULT_SCROLL_DELAY: u64 = 80;
/// Wait after clicking a left-side category (ms).
pub const DEFAULT_CATEGORY_DELAY: u64 = 400;

/// Achievement scanner configuration.
///
/// Timing is owned here on purpose: backpack scroll/tab delays must not leak
/// into the achievement list, which is a different UI with different settle
/// times.
#[derive(Clone, Debug)]
pub struct GoodAchievementScannerConfig {
    pub verbose: bool,
    pub ocr_backend: String,
    /// Wait after a list scroll tick, before the next capture (ms).
    pub scroll_delay: u64,
    /// Wait after clicking a left-side category (ms).
    pub category_delay: u64,
    pub continue_on_failure: bool,
    pub log_progress: bool,
    pub dump_images: bool,
    /// Stop after this many completed achievements (0 = unlimited).
    pub max_count: usize,
}

impl Default for GoodAchievementScannerConfig {
    fn default() -> Self {
        Self {
            verbose: false,
            ocr_backend: "ppocrv4".to_string(),
            scroll_delay: DEFAULT_SCROLL_DELAY,
            category_delay: DEFAULT_CATEGORY_DELAY,
            continue_on_failure: false,
            log_progress: false,
            dump_images: false,
            max_count: 0,
        }
    }
}
