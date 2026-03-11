use clap::Args;

use super::super::good_common::constants::*;

#[derive(Clone, Debug, Args)]
pub struct GoodCharacterScannerConfig {
    /// Show detailed character scan info
    #[arg(long = "good-char-verbose", default_value_t = false)]
    pub verbose: bool,

    /// OCR backend for character scanning (ppocrv5 or ppocrv3)
    #[arg(long = "good-char-ocr-backend", default_value = "ppocrv5")]
    pub ocr_backend: String,

    /// Delay after switching character tabs (ms)
    #[arg(long = "good-char-tab-delay", default_value_t = DEFAULT_DELAY_CHAR_TAB_SWITCH)]
    pub tab_delay: u64,

    /// Delay when opening character screen (ms)
    #[arg(long = "good-char-open-delay", default_value_t = DEFAULT_DELAY_OPEN_SCREEN)]
    pub open_delay: u64,

    /// Continue scanning on individual character failures
    #[arg(long = "good-char-continue-on-failure", default_value_t = false)]
    pub continue_on_failure: bool,

    /// Log each character as it's scanned
    #[arg(long = "good-char-log-progress", default_value_t = false)]
    pub log_progress: bool,

    /// Dump OCR region images for debugging
    #[arg(long = "good-char-dump-images", default_value_t = false)]
    pub dump_images: bool,

    /// Max characters to scan (0 = unlimited)
    #[arg(id = "good-char-max-count", long = "good-char-max-count", default_value_t = 0)]
    pub max_count: usize,
}
