use yas::positioning::{Pos, Rect};

#[derive(Clone, yas_derive::YasWindowInfo, Debug)]
pub struct CharacterScannerWindowInfo {
    /// Character name region (top-left of character screen, shows "Element / Name")
    #[window_info(rename = "genshin_character_name_rect")]
    pub name_rect: Rect<f64>,

    /// Character level region (shows "Lv.XX/YY")
    #[window_info(rename = "genshin_character_level_rect")]
    pub level_rect: Rect<f64>,

    /// Attributes tab click position
    #[window_info(rename = "genshin_character_tab_attributes_pos")]
    pub tab_attributes_pos: Pos<f64>,

    /// Constellation tab click position
    #[window_info(rename = "genshin_character_tab_constellation_pos")]
    pub tab_constellation_pos: Pos<f64>,

    /// Talents tab click position
    #[window_info(rename = "genshin_character_tab_talents_pos")]
    pub tab_talents_pos: Pos<f64>,

    /// Constellation 1 click position
    #[window_info(rename = "genshin_character_constellation_1_pos")]
    pub constellation_1_pos: Pos<f64>,

    /// Constellation 2 click position
    #[window_info(rename = "genshin_character_constellation_2_pos")]
    pub constellation_2_pos: Pos<f64>,

    /// Constellation 3 click position
    #[window_info(rename = "genshin_character_constellation_3_pos")]
    pub constellation_3_pos: Pos<f64>,

    /// Constellation 4 click position
    #[window_info(rename = "genshin_character_constellation_4_pos")]
    pub constellation_4_pos: Pos<f64>,

    /// Constellation 5 click position
    #[window_info(rename = "genshin_character_constellation_5_pos")]
    pub constellation_5_pos: Pos<f64>,

    /// Constellation 6 click position
    #[window_info(rename = "genshin_character_constellation_6_pos")]
    pub constellation_6_pos: Pos<f64>,

    /// Region to check if constellation is activated (sample color near "Activate" button)
    #[window_info(rename = "genshin_character_constellation_activate_rect")]
    pub constellation_activate_rect: Rect<f64>,

    /// Talent auto attack click position
    #[window_info(rename = "genshin_character_talent_auto_pos")]
    pub talent_auto_pos: Pos<f64>,

    /// Talent elemental skill click position
    #[window_info(rename = "genshin_character_talent_skill_pos")]
    pub talent_skill_pos: Pos<f64>,

    /// Talent elemental burst click position
    #[window_info(rename = "genshin_character_talent_burst_pos")]
    pub talent_burst_pos: Pos<f64>,

    /// Talent level OCR region (shown after clicking a talent)
    #[window_info(rename = "genshin_character_talent_level_rect")]
    pub talent_level_rect: Rect<f64>,

    /// Next character arrow click position
    #[window_info(rename = "genshin_character_next_pos")]
    pub next_pos: Pos<f64>,
}
