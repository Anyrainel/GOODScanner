use clap::Args;

use super::super::good_common::constants::*;

#[derive(Clone, Debug, Args)]
pub struct GoodWeaponScannerConfig {
    /// Minimum weapon rarity to keep (3-5)
    #[arg(long = "good-weapon-min-rarity", default_value_t = 3)]
    pub min_rarity: i32,

    /// Show detailed weapon scan info
    #[arg(long = "good-weapon-verbose", default_value_t = false)]
    pub verbose: bool,

    /// OCR backend (ppocrv5 or ppocrv3)
    #[arg(long = "good-weapon-ocr-backend", default_value = "ppocrv5")]
    pub ocr_backend: String,

    /// Delay per grid item (ms)
    #[arg(long = "good-weapon-grid-delay", default_value_t = DEFAULT_DELAY_GRID_ITEM)]
    pub delay_grid_item: u64,

    /// Delay after scrolling (ms)
    #[arg(long = "good-weapon-scroll-delay", default_value_t = DEFAULT_DELAY_SCROLL)]
    pub delay_scroll: u64,

    /// Delay after switching inventory tab (ms)
    #[arg(long = "good-weapon-tab-delay", default_value_t = DEFAULT_DELAY_INV_TAB_SWITCH)]
    pub delay_tab: u64,

    /// Delay when opening screen (ms)
    #[arg(long = "good-weapon-open-delay", default_value_t = DEFAULT_DELAY_OPEN_SCREEN)]
    pub open_delay: u64,

    /// Continue scanning on failures
    #[arg(long = "good-weapon-continue-on-failure", default_value_t = false)]
    pub continue_on_failure: bool,

    /// Log each weapon as it's scanned
    #[arg(long = "good-weapon-log-progress", default_value_t = false)]
    pub log_progress: bool,

    /// Dump OCR region images for debugging
    #[arg(long = "good-weapon-dump-images", default_value_t = false)]
    pub dump_images: bool,

    /// Max weapons to scan (0 = unlimited)
    #[arg(id = "good-weapon-max-count", long = "good-weapon-max-count", default_value_t = 0)]
    pub max_count: usize,
}
