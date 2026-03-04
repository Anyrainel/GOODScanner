#[derive(Clone, clap::Args)]
pub struct GenshinCharacterScannerConfig {
    /// Show verbose character scan info
    #[arg(id = "char-verbose", long = "char-verbose", help = "显示角色扫描详细信息")]
    pub verbose: bool,

    /// OCR backend for character scanning
    #[arg(id = "char-ocr-backend", long = "char-ocr-backend", help = "角色OCR后端", default_value = "ppocrv5")]
    pub ocr_backend: String,

    /// Delay between character switches (ms)
    #[arg(id = "char-delay", long = "char-delay", help = "切换角色后的等待时间(ms)", default_value_t = 500)]
    pub delay: u32,

    /// Delay after clicking tabs (ms)
    #[arg(id = "char-tab-delay", long = "char-tab-delay", help = "切换标签后的等待时间(ms)", default_value_t = 700)]
    pub tab_delay: u32,

    /// Delay after clicking constellation/talent nodes (ms)
    #[arg(id = "char-node-delay", long = "char-node-delay", help = "点击命座/天赋节点后的等待时间(ms)", default_value_t = 550)]
    pub node_delay: u32,

    /// Save captured images for debugging
    #[arg(id = "char-save-images", long = "char-save-images", help = "保存角色识别图片")]
    pub save_images: bool,
}
