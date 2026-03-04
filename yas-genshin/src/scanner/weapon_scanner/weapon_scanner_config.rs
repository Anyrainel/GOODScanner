#[derive(Clone, clap::Args)]
pub struct GenshinWeaponScannerConfig {
    /// Items with stars less than this will be ignored
    #[arg(id = "weapon-min-star", long = "weapon-min-star", help = "武器最小星级", default_value_t = 3)]
    pub min_star: i32,

    /// it will output very verbose messages
    #[arg(id = "weapon-verbose", long = "weapon-verbose", help = "显示武器详细信息")]
    pub verbose: bool,

    /// the exact amount to scan
    #[arg(id = "weapon-number", long = "weapon-number", help = "指定武器数量", default_value_t = -1)]
    pub number: i32,

    /// save captured images for debugging
    #[arg(id = "weapon-save-images", long = "weapon-save-images", help = "保存武器识别图片")]
    pub save_images: bool,

    /// 选择OCR后端
    #[arg(id = "weapon-ocr-backend", long = "weapon-ocr-backend", help = "武器OCR后端", default_value = "ppocrv5")]
    pub ocr_backend: String,

    /// 每次切换武器后的额外等待时间(ms)
    #[arg(id = "weapon-delay", long = "weapon-delay", help = "每次切换武器后的额外等待时间(ms)", default_value_t = 20)]
    pub delay: u32,

    /// Ignore duplicated items
    #[arg(id = "weapon-ignore-dup", long = "weapon-ignore-dup", help = "忽略重复武器")]
    pub ignore_dup: bool,
}
