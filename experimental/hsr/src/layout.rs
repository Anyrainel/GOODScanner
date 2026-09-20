//! 16:9 click and crop geometry for the live HSR screenshot scanner.
//!
//! Coordinates come from kel-z/HSR-Scanner `screenshot.py` / nav data. That
//! repo stores some panel crops as `(x0, y0, x1, y1)`; this module converts
//! those to `(x, y, w, h)` once so scanner and OCR code never mix conventions.
//! Window-level screenshot entries in kel-z are already `(x, y, w, h)`.

use crate::vision::{NormRect, Point};

/// Convert kel-z panel crops stored as `(x0, y0, x1, y1)` into `(x, y, w, h)`.
const fn xyxy(x0: f64, y0: f64, x1: f64, y1: f64) -> NormRect {
    NormRect::new(x0, y0, x1 - x0, y1 - y0)
}

pub const STATS_PANEL: NormRect = NormRect::new(0.72, 0.09, 0.25, 0.78);
pub const QUANTITY: NormRect = NormRect::new(
    0.788_291_666_666_666_7,
    0.032_407_407_407_407_406,
    0.14,
    0.06,
);

pub const LIGHT_CONE_TAB: Point = Point::new(0.38, 0.06);
pub const GEAR_TAB: Point = Point::new(0.43, 0.06);
pub const FIRST_ITEM: Point = Point::new(0.071, 0.26);

pub const DETAILS_BUTTON: Point = Point::new(0.13, 0.143);
pub const TRACES_BUTTON: Point = Point::new(0.13, 0.315);
pub const EIDOLONS_BUTTON: Point = Point::new(0.13, 0.49);

/// kel-z `_screenshot_traces` crop size for visible skill level text (`6/10`).
pub const TRACE_LEVEL_SIZE: (f64, f64) = (0.04, 0.028);

#[derive(Clone, Copy)]
pub struct PathTraces {
    pub skills: &'static [(&'static str, f64, f64)],
    pub unlocks: &'static [(&'static str, f64, f64)],
}

pub fn skill_level_rect(x: f64, y: f64) -> NormRect {
    NormRect::new(x, y, TRACE_LEVEL_SIZE.0, TRACE_LEVEL_SIZE.1)
}

/// Accepts GIlore path keys (`Warrior`) and Fribbels names (`Destruction`).
pub fn traces_for_path(path: &str) -> Option<PathTraces> {
    Some(match path {
        "Rogue" | "Hunt" => HUNT_TRACES,
        "Mage" | "Erudition" => ERUDITION_TRACES,
        "Shaman" | "Harmony" => HARMONY_TRACES,
        "Knight" | "Preservation" => PRESERVATION_TRACES,
        "Warrior" | "Destruction" => DESTRUCTION_TRACES,
        "Warlock" | "Nihility" => NIHILITY_TRACES,
        "Priest" | "Abundance" => ABUNDANCE_TRACES,
        "Memory" | "Remembrance" => REMEMBRANCE_TRACES,
        "Elation" => ELATION_TRACES,
        _ => return None,
    })
}

const HUNT_TRACES: PathTraces = PathTraces {
    skills: &[
        ("basic", 0.502_171_875, 0.534_722_222_222_222_2),
        ("skill", 0.655_468_75, 0.534_722_222_222_222_2),
        ("ult", 0.579_296_875, 0.600_694_444_444_444_4),
        ("talent", 0.579_296_875, 0.461_5),
    ],
    unlocks: &[
        (
            "ability_1",
            0.502_083_333_333_333_3,
            0.683_333_333_333_333_3,
        ),
        ("ability_2", 0.663_5, 0.687_9),
        ("ability_3", 0.582_8, 0.316_6),
        ("stat_1", 0.589_58, 0.818_5),
        ("stat_2", 0.451, 0.599),
        ("stat_3", 0.396_3, 0.503_7),
        ("stat_4", 0.725_5, 0.6),
        ("stat_5", 0.725_5, 0.596_29),
        ("stat_6", 0.780_7, 0.503_7),
        ("stat_7", 0.723_9, 0.380_5),
        ("stat_8", 0.588_54, 0.225_9),
        ("stat_9", 0.499_48, 0.252_77),
        ("stat_10", 0.678_6, 0.252_77),
    ],
};
const ERUDITION_TRACES: PathTraces = PathTraces {
    skills: &[
        ("basic", 0.507_687_5, 0.588_888_888_888_888_9),
        ("skill", 0.651_171_875, 0.588_888_888_888_888_9),
        ("ult", 0.579_296_875, 0.588_888_888_888_888_9),
        ("talent", 0.579_296_875, 0.439_583_333_333_333_3),
    ],
    unlocks: &[
        ("ability_1", 0.451_56, 0.541_666),
        ("ability_2", 0.715_625, 0.538_89),
        ("ability_3", 0.582_8, 0.22),
        ("stat_1", 0.516_1, 0.750_9),
        ("stat_2", 0.398_9, 0.547_2),
        ("stat_3", 0.415_6, 0.655_5),
        ("stat_4", 0.415_6, 0.434_26),
        ("stat_5", 0.774_4, 0.545_37),
        ("stat_6", 0.759_375, 0.655_5),
        ("stat_7", 0.759_375, 0.435_2),
        ("stat_8", 0.498_4, 0.25),
        ("stat_9", 0.678_64, 0.248_1),
        ("stat_10", 0.658_8, 0.745_37),
    ],
};
const HARMONY_TRACES: PathTraces = PathTraces {
    skills: &[
        ("basic", 0.508_203_125, 0.549_305_555_555_555_6),
        ("skill", 0.650_562_5, 0.549_305_555_555_555_6),
        ("ult", 0.579_078_125, 0.645_833_333_333_333_4),
        ("talent", 0.579_078_125, 0.524_305_555_555_555_6),
    ],
    unlocks: &[
        ("ability_1", 0.414, 0.563_89),
        ("ability_2", 0.752_083, 0.563_89),
        ("ability_3", 0.583_8, 0.331_5),
        ("stat_1", 0.590_1, 0.819),
        ("stat_2", 0.382_8, 0.476_85),
        ("stat_3", 0.446_875, 0.409_25),
        ("stat_4", 0.509_895, 0.794_44),
        ("stat_5", 0.726_56, 0.676_85),
        ("stat_6", 0.667_18, 0.639_81),
        ("stat_7", 0.670_31, 0.794_44),
        ("stat_8", 0.591_1, 0.226_85),
        ("stat_9", 0.504_68, 0.252_77),
        ("stat_10", 0.676_04, 0.253_7),
    ],
};
const PRESERVATION_TRACES: PathTraces = PathTraces {
    skills: &[
        ("basic", 0.503_515_625, 0.606_638_888_888_888_8),
        ("skill", 0.654_125, 0.606_944_444_444_444_4),
        ("ult", 0.579_687_5, 0.588_194_444_444_444_5),
        ("talent", 0.579_687_5, 0.461_5),
    ],
    unlocks: &[
        ("ability_1", 0.497_395, 0.803_703_7),
        ("ability_2", 0.665_625, 0.803_703),
        ("ability_3", 0.582_291_6, 0.321_296_296),
        ("stat_1", 0.589_583, 0.8),
        ("stat_2", 0.435_416, 0.662_96),
        ("stat_3", 0.383_33, 0.551_85),
        ("stat_4", 0.452_08, 0.444_44),
        ("stat_5", 0.743_229, 0.664_81),
        ("stat_6", 0.795_833, 0.550_925),
        ("stat_7", 0.727_083, 0.446_296_2),
        ("stat_8", 0.589_583, 0.229_629),
        ("stat_9", 0.499_479, 0.255_555),
        ("stat_10", 0.679_687_5, 0.253_7),
    ],
};
const DESTRUCTION_TRACES: PathTraces = PathTraces {
    skills: &[
        ("basic", 0.491_796_875, 0.568_75),
        ("skill", 0.664_062_5, 0.568_75),
        ("ult", 0.579_734_375, 0.588_888_888_888_888_9),
        ("talent", 0.579_734_375, 0.462_5),
    ],
    unlocks: &[
        (
            "ability_1",
            0.490_104_166_666_666_7,
            0.704_629_629_629_629_6,
        ),
        ("ability_2", 0.673_437_5, 0.7),
        ("ability_3", 0.596_875, 0.302_222_222_222_222_2),
        ("stat_1", 0.589_062_5, 0.813_889),
        ("stat_2", 0.439_583_3, 0.634_259_25),
        ("stat_3", 0.396_875, 0.536_666_666_666_666_6),
        ("stat_4", 0.437_5, 0.418_51),
        ("stat_5", 0.739_583_3, 0.637_962_96),
        ("stat_6", 0.792_187_5, 0.546_296_296_296_296_3),
        ("stat_7", 0.741_666, 0.420_37),
        ("stat_8", 0.589_583_33, 0.229_629),
        ("stat_9", 0.500_52, 0.256_481),
        ("stat_10", 0.679_166_6, 0.255_55),
    ],
};
const NIHILITY_TRACES: PathTraces = PathTraces {
    skills: &[
        ("basic", 0.498_046_875, 0.517_361_111_111_111_2),
        ("skill", 0.658_984_375, 0.517_361_111_111_111_2),
        ("ult", 0.579_734_375, 0.504_861_111_111_111_1),
        ("talent", 0.579_734_375, 0.387_5),
    ],
    unlocks: &[
        ("ability_1", 0.436_871_875, 0.425_694_444_4),
        ("ability_2", 0.731_640_625, 0.421_527_7),
        ("ability_3", 0.585_937_5, 0.222_916_66),
        ("stat_1", 0.590_234_375, 0.703_472_2),
        ("stat_2", 0.381_25, 0.545_833_3),
        ("stat_3", 0.435_937_5, 0.656_944_4),
        ("stat_4", 0.489_453_125, 0.768_75),
        ("stat_5", 0.798_828, 0.546_527_7),
        ("stat_6", 0.744_921_875, 0.656_944_4),
        ("stat_7", 0.691_796_875, 0.768_055_5),
        ("stat_8", 0.500_781_25, 0.252_777_7),
        ("stat_9", 0.680_859_375, 0.252_577_77),
        ("stat_10", 0.590_625, 0.807_638_88),
    ],
};
const ABUNDANCE_TRACES: PathTraces = PathTraces {
    skills: &[
        ("basic", 0.506_640_625, 0.567_361_111_111_111_1),
        ("skill", 0.651_953_125, 0.567_361_111_111_111_1),
        ("ult", 0.579_687_5, 0.593_75),
        ("talent", 0.579_687_5, 0.462_5),
    ],
    unlocks: &[
        ("ability_1", 0.472_265, 0.720_138),
        ("ability_2", 0.694_140, 0.718_75),
        ("ability_3", 0.584_375, 0.215_972_3),
        ("stat_1", 0.630_859, 0.802_777),
        ("stat_2", 0.749_218, 0.616_666),
        ("stat_3", 0.778_515, 0.527_777),
        ("stat_4", 0.722_656, 0.435_416_6),
        ("stat_5", 0.433_203_125, 0.618_055),
        ("stat_6", 0.403_906_25, 0.527_083_33),
        ("stat_7", 0.459_765_625, 0.435_416_66),
        ("stat_8", 0.678_906_25, 0.257_638_8),
        ("stat_9", 0.504_296_875, 0.258_333_3),
        ("stat_10", 0.550_390_625, 0.804_166_66),
    ],
};
const REMEMBRANCE_TRACES: PathTraces = PathTraces {
    skills: &[
        ("basic", 0.517_312_5, 0.697_333_333_333_333),
        ("skill", 0.661_328_125, 0.697_222_222_222_222_2),
        ("ult", 0.589_453_125, 0.734_027_777_777_777_7),
        ("talent", 0.702_734_375, 0.569_444_444_444_444_4),
        ("memosprite_skill", 0.59, 0.548_611_111_111_111_2),
        ("memosprite_talent", 0.59, 0.386_277_777_777_777_8),
    ],
    unlocks: &[
        ("ability_1", 0.792_187_5, 0.514_583_333_333_333_3),
        ("ability_2", 0.604_296_875, 0.800_694_444_444_444_5),
        ("ability_3", 0.526_171_875, 0.378_472_222_222_222_2),
        ("stat_1", 0.781_25, 0.645_138_888_888_888_9),
        ("stat_2", 0.774_218_75, 0.391_666_666_666_666_6),
        ("stat_3", 0.410_937_5, 0.519_444_444_444_444_5),
        ("stat_4", 0.431_25, 0.643_75),
        ("stat_5", 0.432_031_25, 0.391_666_666_666_666_6),
        ("stat_6", 0.534_375, 0.790_277_777_777_777_7),
        ("stat_7", 0.675, 0.791_666_666_666_666_6),
        ("stat_8", 0.494_921_875, 0.272_916_666_666_666_64),
        ("stat_9", 0.563_671_875, 0.218_75),
        ("stat_10", 0.644_531_25, 0.216_666_666_666_666_67),
    ],
};
const ELATION_TRACES: PathTraces = PathTraces {
    skills: &[
        ("basic", 0.505_729_166_666_666_7, 0.35),
        ("skill", 0.652_083_333_333_333_3, 0.35),
        ("ult", 0.577_604_166_666_666_7, 0.457_407_407_407_407_43),
        ("talent", 0.577_604_166_666_666_7, 0.594_444_444_444_444_4),
        ("elation", 0.577_604_166_666_666_7, 0.299_074_074_074_074_05),
    ],
    unlocks: &[
        (
            "ability_1",
            0.583_333_333_333_333_4,
            0.831_481_481_481_481_5,
        ),
        (
            "ability_2",
            0.436_458_333_333_333_34,
            0.383_333_333_333_333_36,
        ),
        (
            "ability_3",
            0.730_208_333_333_333_3,
            0.384_259_259_259_259_24,
        ),
        ("stat_1", 0.523_437_5, 0.648_148_148_148_148_1),
        ("stat_2", 0.666_666_666_666_666_6, 0.648_148_148_148_148_1),
        ("stat_3", 0.531_770_833_333_333_3, 0.832_407_407_407_407_4),
        ("stat_4", 0.654_687_5, 0.833_333_333_333_333_4),
        ("stat_5", 0.413_541_666_666_666_64, 0.511_111_111_111_111_1),
        ("stat_6", 0.429_166_666_666_666_64, 0.631_481_481_481_481_5),
        ("stat_7", 0.478_645_833_333_333_3, 0.559_259_259_259_259_2),
        ("stat_8", 0.771_875, 0.510_185_185_185_185_2),
        ("stat_9", 0.760_416_666_666_666_6, 0.632_407_407_407_407_4),
        ("stat_10", 0.708_854_166_666_666_7, 0.561_111_111_111_111_1),
    ],
};

pub const CHARACTER_NAME: NormRect = NormRect::new(0.0656, 0.055, 0.185, 0.036);
pub const CHARACTER_LEVEL: NormRect = NormRect::new(0.772, 0.218, 0.085, 0.040);

/// Static HUD strips used to decide that a menu has settled. HSR character and
/// inventory screens animate a 3D model and starfield in the center, so
/// full-frame similarity can never succeed there.
pub const UI_CHROME_LEFT: NormRect = NormRect::new(0.0, 0.0, 0.20, 1.0);
pub const UI_CHROME_RIGHT: NormRect = NormRect::new(0.74, 0.0, 0.26, 1.0);

pub const RELIC_NAME: NormRect = xyxy(0.0, 0.02, 0.82, 0.10);
pub const RELIC_LEVEL: NormRect = xyxy(0.03, 0.25, 0.28, 0.32);
pub const RELIC_RARITY: NormRect = xyxy(0.07, 0.15, 0.2, 0.22);
pub const RELIC_MAIN_NAME: NormRect = xyxy(0.11, 0.358, 0.7, 0.4);
pub const RELIC_MAIN_VALUE: NormRect = xyxy(0.775, 0.358, 0.975, 0.4);
/// Wider than kel-z's text-only equipped strip so Chinese 「装备中」 plus the
/// portrait stay in one dump crop. OCR still keys off the label, not the face.
pub const RELIC_EQUIPPED: NormRect = xyxy(0.18, 0.905, 0.82, 0.975);
pub const RELIC_LOCK: NormRect = xyxy(
    0.858_333_333_333_333_3,
    0.181_710_213_776_722_08,
    0.937_5,
    0.226_840_855_106_888_37,
);
pub const RELIC_DISCARD: NormRect = xyxy(0.865, 0.253, 0.935, 0.293);

pub const LIGHT_CONE_NAME: NormRect = xyxy(0.0, 0.0, 1.0, 0.09);
pub const LIGHT_CONE_LEVEL: NormRect = xyxy(0.13, 0.32, 0.35, 0.37);
/// kel-z's tiny box sits on skill percent text in the Chinese panel. This
/// strip is the 「叠影N阶」 row confirmed from live dumps.
pub const LIGHT_CONE_SUPERIMPOSITION: NormRect = xyxy(0.02, 0.40, 0.50, 0.48);
pub const LIGHT_CONE_EQUIPPED: NormRect = RELIC_EQUIPPED;
pub const LIGHT_CONE_LOCK: NormRect = xyxy(0.896, 0.321, 0.97, 0.365);

const RELIC_SUB_NAMES: NormRect = xyxy(0.11, 0.4, 0.5, 0.58);
const RELIC_SUB_VALUES: NormRect = xyxy(0.775, 0.4, 0.975, 0.58);
const RELIC_SUB_LINES: f64 = 4.0;

pub fn relic_sub_name(index: usize) -> NormRect {
    split_vertical_line(RELIC_SUB_NAMES, index)
}

pub fn relic_sub_value(index: usize) -> NormRect {
    split_vertical_line(RELIC_SUB_VALUES, index)
}

fn split_vertical_line(block: NormRect, index: usize) -> NormRect {
    let height = block.height / RELIC_SUB_LINES;
    NormRect::new(
        block.x,
        block.y + index as f64 * height,
        block.width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kelz_xyxy_panel_crops_convert_to_positive_boxes() {
        assert!(RELIC_LEVEL.width > 0.0 && RELIC_LEVEL.height > 0.0);
        assert!((RELIC_LEVEL.width - 0.25).abs() < 1e-9);
        assert!((RELIC_LEVEL.height - 0.07).abs() < 1e-9);
        assert!((LIGHT_CONE_LEVEL.width - 0.22).abs() < 1e-9);
        assert!(LIGHT_CONE_SUPERIMPOSITION.width > 0.2);
        assert!(LIGHT_CONE_SUPERIMPOSITION.height > 0.0);
        assert!((RELIC_LOCK.width - (0.937_5 - 0.858_333_333_333_333_3)).abs() < 1e-9);
        assert!((RELIC_LOCK.y - 0.181_710_213_776_722_08).abs() < 1e-9);
        assert!((relic_sub_name(0).height - RELIC_SUB_NAMES.height / 4.0).abs() < 1e-9);
        assert!(
            (relic_sub_name(3).y + relic_sub_name(3).height
                - RELIC_SUB_NAMES.y
                - RELIC_SUB_NAMES.height)
                .abs()
                < 1e-9
        );
        assert!((TRACES_BUTTON.y - 0.315).abs() < 1e-9);
        let hunt = traces_for_path("Hunt").expect("Hunt traces");
        assert_eq!(hunt.skills.len(), 4);
        assert_eq!(hunt.unlocks.len(), 13);
        assert!(traces_for_path("Memory").is_some());
        assert!(skill_level_rect(0.5, 0.5).width > 0.0);
    }
}
