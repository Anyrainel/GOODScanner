/// Character scanner configuration.
///
/// All fields are set by the orchestrator (`cli.rs`) from global CLI flags
/// and `good_config.json`. This struct has no clap derives.
#[derive(Clone, Debug)]
pub struct GoodCharacterScannerConfig {
    pub verbose: bool,
    pub ocr_backend: String,
    pub tab_delay: u64,
    pub open_delay: u64,
    pub continue_on_failure: bool,
    pub log_progress: bool,
    pub dump_images: bool,
    pub max_count: usize,
}

impl Default for GoodCharacterScannerConfig {
    fn default() -> Self {
        Self {
            verbose: false,
            ocr_backend: "ppocrv4".to_string(),
            tab_delay: 400,
            open_delay: 1200,
            continue_on_failure: false,
            log_progress: false,
            dump_images: false,
            max_count: 0,
        }
    }
}
