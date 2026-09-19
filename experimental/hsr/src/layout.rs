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
pub const EIDOLONS_BUTTON: Point = Point::new(0.13, 0.49);

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
    }
}
