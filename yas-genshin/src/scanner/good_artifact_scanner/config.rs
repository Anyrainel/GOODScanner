use clap::Args;

use super::super::good_common::constants::*;

#[derive(Clone, Debug, Args)]
pub struct GoodArtifactScannerConfig {
    /// Minimum artifact rarity to keep (4-5)
    #[arg(long = "good-artifact-min-rarity", default_value_t = 4)]
    pub min_rarity: i32,

    /// Show detailed artifact scan info
    #[arg(long = "good-artifact-verbose", default_value_t = false)]
    pub verbose: bool,

    /// OCR backend (ppocrv5 or ppocrv3)
    #[arg(long = "good-artifact-ocr-backend", default_value = "ppocrv5")]
    pub ocr_backend: String,

    /// Secondary OCR backend for dual-engine substat/level recognition.
    /// Used alongside the main backend; results are merged.
    #[arg(long = "good-artifact-substat-ocr-backend", default_value = "ppocrv4")]
    pub substat_ocr_backend: String,

    /// Delay per grid item (ms)
    #[arg(long = "good-artifact-grid-delay", default_value_t = DEFAULT_DELAY_GRID_ITEM)]
    pub delay_grid_item: u64,

    /// Delay after scrolling (ms)
    #[arg(long = "good-artifact-scroll-delay", default_value_t = DEFAULT_DELAY_SCROLL)]
    pub delay_scroll: u64,

    /// Delay after switching inventory tab (ms)
    #[arg(long = "good-artifact-tab-delay", default_value_t = DEFAULT_DELAY_INV_TAB_SWITCH)]
    pub delay_tab: u64,

    /// Delay when opening screen (ms)
    #[arg(long = "good-artifact-open-delay", default_value_t = DEFAULT_DELAY_OPEN_SCREEN)]
    pub open_delay: u64,

    /// Continue scanning on failures
    #[arg(long = "good-artifact-continue-on-failure", default_value_t = false)]
    pub continue_on_failure: bool,

    /// Log each artifact as it's scanned
    #[arg(long = "good-artifact-log-progress", default_value_t = false)]
    pub log_progress: bool,

    /// Dump OCR region images for debugging
    #[arg(long = "good-artifact-dump-images", default_value_t = false)]
    pub dump_images: bool,

    /// Max artifacts to scan (0 = unlimited)
    #[arg(id = "good-artifact-max-count", long = "good-artifact-max-count", default_value_t = 0)]
    pub max_count: usize,
}
