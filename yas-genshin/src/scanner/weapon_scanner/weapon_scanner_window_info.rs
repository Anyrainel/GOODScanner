use yas::positioning::{Pos, Rect, Size};

#[derive(Clone, yas_derive::YasWindowInfo, Debug)]
pub struct WeaponScannerWindowInfo {
    /// the position of weapon title relative to window
    #[window_info(rename = "genshin_weapon_title_rect")]
    pub title_rect: Rect<f64>,

    /// the level of the weapon relative to window
    #[window_info(rename = "genshin_weapon_level_rect")]
    pub level_rect: Rect<f64>,

    /// the refinement of the weapon relative to window
    #[window_info(rename = "genshin_weapon_refinement_rect")]
    pub refinement_rect: Rect<f64>,

    /// equip status of the weapon relative to window
    #[window_info(rename = "genshin_weapon_item_equip_rect")]
    pub equip_rect: Rect<f64>,

    /// the sample position of star, relative to window
    #[window_info(rename = "genshin_weapon_star_pos")]
    pub star_pos: Pos<f64>,

    /// the whole panel of the weapon, relative to window
    #[window_info(rename = "genshin_repository_panel_rect")]
    pub panel_rect: Rect<f64>,

    /// how many columns in this layout
    #[window_info(rename = "genshin_repository_item_col")]
    pub col: i32,

    /// how many rows in this layout
    #[window_info(rename = "genshin_repository_item_row")]
    pub row: i32,

    #[window_info(rename = "genshin_repository_item_gap_size")]
    pub item_gap_size: Size<f64>,

    #[window_info(rename = "genshin_repository_item_size")]
    pub item_size: Size<f64>,

    #[window_info(rename = "genshin_repository_scan_margin_pos")]
    pub scan_margin_pos: Pos<f64>,

    #[window_info(rename = "genshin_repository_lock_pos")]
    pub lock_pos: Pos<f64>,

    /// the count of weapons relative to window
    #[window_info(rename = "genshin_weapon_item_count_rect")]
    pub item_count_rect: Rect<f64>,
}
