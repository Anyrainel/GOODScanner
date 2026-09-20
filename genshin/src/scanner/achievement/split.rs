//! Pixel helpers for the in-game achievement list.
//!
//! [`detect_list_rect`] finds the right-hand cream card panel.
//! [`split_row_bands`] cuts that panel on the hairline between cards.
//! [`crop_frac`] takes title/status crops inside one row.

use image::{GenericImageView, Rgb, RgbImage};

use super::layout::{
    CATEGORY_PROBE_X0, CATEGORY_PROBE_X1, CATEGORY_SELECTED_MAX_H, CATEGORY_SELECTED_MIN_H,
    LIST_MIN_HEIGHT_RATIO, LIST_MIN_WIDTH_RATIO, LIST_PANEL_GRAY, LIST_PROBE_X0, LIST_PROBE_X1,
    LIST_ROW_BRIGHT_RATIO, LIST_RUN_GAP, LIST_UNCHANGED_MEAN_DELTA, LIST_X_GAP, MIN_ROW_HEIGHT,
    PARTIAL_ROW_HEIGHT_RATIO, ROW_DIVIDER_MAX,
};

/// Axis-aligned rectangle in image pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// One achievement card sliced from the list panel.
#[derive(Clone, Debug)]
pub struct RowBand {
    pub rect: PixelRect,
}

/// Detect the right-hand cream achievement list in a full-window capture.
///
/// The in-game list is not a solid bright blob: card icons punch dark holes, so
/// a strict consecutive-run detector misses it. We probe the right side of the
/// window, allow short gaps, then take the longest cream span on a mid row
/// (that excludes the dark left-hand category sidebar).
pub fn detect_list_rect(image: &RgbImage) -> Option<PixelRect> {
    let width = image.width();
    let height = image.height();
    if width < 32 || height < 32 {
        return None;
    }

    let x0 = ((width as f32) * LIST_PROBE_X0) as u32;
    let x1 = ((width as f32) * LIST_PROBE_X1) as u32;
    let span = x1.saturating_sub(x0).max(1);
    let mut row_bright = vec![0.0f32; height as usize];
    for y in 0..height {
        let mut count = 0u32;
        for x in x0..x1 {
            if luma(image.get_pixel(x, y)) >= LIST_PANEL_GRAY {
                count += 1;
            }
        }
        row_bright[y as usize] = count as f32 / span as f32;
    }

    let (y0, panel_h) = longest_run_with_gaps(&row_bright, LIST_ROW_BRIGHT_RATIO, LIST_RUN_GAP)?;
    if (panel_h as f32) < height as f32 * LIST_MIN_HEIGHT_RATIO {
        return None;
    }

    let mid_y = y0 + panel_h / 2;
    let (min_x, panel_w) = longest_cream_x_span(image, mid_y, LIST_X_GAP)?;
    if (panel_w as f32) < width as f32 * LIST_MIN_WIDTH_RATIO {
        return None;
    }

    let inset_x = panel_w / 40;
    let inset_y = panel_h / 80;
    let x = min_x.saturating_add(inset_x);
    let y = y0.saturating_add(inset_y);
    let w = panel_w.saturating_sub(inset_x * 2).max(8);
    let h = panel_h.saturating_sub(inset_y * 2).max(8);
    let w = w.min(width.saturating_sub(x));
    let h = h.min(height.saturating_sub(y));
    Some(PixelRect { x, y, w, h })
}

/// Split a list-panel crop into card rows using the cream-body hairlines.
///
/// Each card is cream (~231). The separator between cards dips to ~206–213.
/// Sampling a column in the text area (not the left-side icons) finds those
/// dips; a short hit near the panel top is the inner edge, not a card.
pub fn split_row_bands(list: &RgbImage, keep_last: bool) -> Vec<RowBand> {
    let width = list.width();
    let height = list.height();
    if width < 16 || height < 16 {
        return Vec::new();
    }

    let sample_x = (width * 3 / 5).min(width - 1);
    let mut dividers: Vec<u32> = Vec::new();
    let mut cluster_start: Option<u32> = None;
    for y in 0..height {
        let v = luma(list.get_pixel(sample_x, y));
        if v <= ROW_DIVIDER_MAX {
            if cluster_start.is_none() {
                cluster_start = Some(y);
            }
        } else if let Some(start) = cluster_start.take() {
            dividers.push(start);
        }
    }
    if let Some(start) = cluster_start {
        dividers.push(start);
    }

    let mut rows = Vec::new();
    let mut prev = 0u32;
    for divider in dividers {
        if divider.saturating_sub(prev) >= MIN_ROW_HEIGHT {
            rows.push(RowBand {
                rect: PixelRect {
                    x: 0,
                    y: prev,
                    w: width,
                    h: divider - prev,
                },
            });
            prev = divider;
        }
    }
    if height.saturating_sub(prev) >= MIN_ROW_HEIGHT {
        rows.push(RowBand {
            rect: PixelRect {
                x: 0,
                y: prev,
                w: width,
                h: height - prev,
            },
        });
    }

    if rows.is_empty() {
        return rows;
    }

    let mut heights: Vec<u32> = rows.iter().map(|r| r.rect.h).collect();
    heights.sort_unstable();
    let median = heights[heights.len() / 2] as f32;

    if !keep_last {
        while rows.len() > 1 {
            let first_h = rows[0].rect.h as f32;
            if first_h < median * PARTIAL_ROW_HEIGHT_RATIO {
                rows.remove(0);
            } else {
                break;
            }
        }
        while rows.len() > 1 {
            let last_h = rows.last().unwrap().rect.h as f32;
            if last_h < median * PARTIAL_ROW_HEIGHT_RATIO {
                rows.pop();
            } else {
                break;
            }
        }
        if let Some(last) = rows.last() {
            if height.saturating_sub(last.rect.y + last.rect.h) < 10 && rows.len() > 1 {
                rows.pop();
            }
        }
    }

    rows.retain(|r| r.rect.h >= 16);
    rows
}

fn longest_run_with_gaps(values: &[f32], threshold: f32, gap: u32) -> Option<(u32, u32)> {
    let mut best = (0u32, 0u32);
    let mut run_start: Option<u32> = None;
    let mut dark = 0u32;
    for (y, value) in values.iter().enumerate() {
        let y = y as u32;
        if *value >= threshold {
            if run_start.is_none() {
                run_start = Some(y);
            }
            dark = 0;
        } else if let Some(start) = run_start {
            dark += 1;
            if dark > gap {
                let end = y - dark;
                let len = end.saturating_sub(start) + 1;
                if len > best.1 {
                    best = (start, len);
                }
                run_start = None;
                dark = 0;
            }
        }
    }
    if let Some(start) = run_start {
        let end = (values.len() as u32).saturating_sub(1).saturating_sub(dark);
        let len = end.saturating_sub(start) + 1;
        if len > best.1 {
            best = (start, len);
        }
    }
    if best.1 == 0 {
        None
    } else {
        Some(best)
    }
}

fn longest_cream_x_span(image: &RgbImage, y: u32, gap: u32) -> Option<(u32, u32)> {
    longest_cream_x_span_between(image, y, 0, image.width(), gap)
}

/// Cream selected row in the left category column.
pub fn detect_selected_category_rect(image: &RgbImage) -> Option<PixelRect> {
    let width = image.width();
    let height = image.height();
    if width < 32 || height < 32 {
        return None;
    }

    let x0 = ((width as f32) * CATEGORY_PROBE_X0) as u32;
    let x1 = ((width as f32) * CATEGORY_PROBE_X1) as u32;
    let span = x1.saturating_sub(x0).max(1);
    let mut row_bright = vec![0.0f32; height as usize];
    for y in 0..height {
        let mut count = 0u32;
        for x in x0..x1 {
            if luma(image.get_pixel(x, y)) >= LIST_PANEL_GRAY {
                count += 1;
            }
        }
        row_bright[y as usize] = count as f32 / span as f32;
    }

    let (y0, panel_h) = longest_run_with_gaps(&row_bright, LIST_ROW_BRIGHT_RATIO, 8)?;
    if panel_h < CATEGORY_SELECTED_MIN_H || panel_h > CATEGORY_SELECTED_MAX_H {
        return None;
    }

    let mid_y = y0 + panel_h / 2;
    let (min_x, panel_w) = longest_cream_x_span_between(image, mid_y, x0, x1, 8)?;
    let inset_x = (panel_w / 12).max(4);
    let inset_y = (panel_h / 8).max(2);
    let x = min_x.saturating_add(inset_x);
    let y = y0.saturating_add(inset_y);
    let w = panel_w.saturating_sub(inset_x * 2).max(8);
    let h = panel_h.saturating_sub(inset_y * 2).max(8);
    Some(PixelRect {
        x,
        y,
        w: w.min(width.saturating_sub(x)),
        h: h.min(height.saturating_sub(y)),
    })
}

fn longest_cream_x_span_between(
    image: &RgbImage,
    y: u32,
    x0: u32,
    x1: u32,
    gap: u32,
) -> Option<(u32, u32)> {
    let width = image.width();
    if y >= image.height() {
        return None;
    }
    let x1 = x1.min(width);
    let mut best = (0u32, 0u32);
    let mut run_start: Option<u32> = None;
    let mut holes = 0u32;
    for x in x0..x1 {
        if luma(image.get_pixel(x, y)) >= LIST_PANEL_GRAY {
            if run_start.is_none() {
                run_start = Some(x);
            }
            holes = 0;
        } else if let Some(start) = run_start {
            holes += 1;
            if holes > gap {
                let end = x - holes;
                let len = end.saturating_sub(start) + 1;
                if len > best.1 {
                    best = (start, len);
                }
                run_start = None;
                holes = 0;
            }
        }
    }
    if let Some(start) = run_start {
        let end = x1.saturating_sub(1).saturating_sub(holes);
        let len = end.saturating_sub(start) + 1;
        if len > best.1 {
            best = (start, len);
        }
    }
    if best.1 == 0 {
        None
    } else {
        Some(best)
    }
}

/// Crop a fractional region out of a row image.
pub fn crop_frac(row: &RgbImage, frac: (f32, f32, f32, f32)) -> Option<RgbImage> {
    let (fx, fy, fw, fh) = frac;
    let x = (row.width() as f32 * fx).round() as u32;
    let y = (row.height() as f32 * fy).round() as u32;
    let w = (row.width() as f32 * fw).round() as u32;
    let h = (row.height() as f32 * fh).round() as u32;
    if w == 0 || h == 0 || x >= row.width() || y >= row.height() {
        return None;
    }
    let w = w.min(row.width() - x);
    let h = h.min(row.height() - y);
    Some(row.view(x, y, w, h).to_image())
}

/// True when two list crops show the same cards (scroll hit the bottom).
///
/// Only the left ~58% is compared. The right side is status/date text that
/// shimmers and would otherwise keep the controller scrolling after the list
/// has already stopped moving.
pub fn list_nearly_equal(a: &RgbImage, b: &RgbImage) -> bool {
    if a.dimensions() != b.dimensions() {
        return false;
    }
    let (w, h) = a.dimensions();
    if w == 0 || h == 0 {
        return false;
    }
    let x_limit = ((w as f32) * 0.58).round().max(1.0) as u32;
    let step_x = (x_limit / 32).max(1) as usize;
    let step_y = (h / 32).max(1) as usize;
    let mut sum = 0u64;
    let mut n = 0u64;
    for y in (0..h).step_by(step_y) {
        for x in (0..x_limit).step_by(step_x) {
            let pa = a.get_pixel(x, y);
            let pb = b.get_pixel(x, y);
            sum += pa[0].abs_diff(pb[0]) as u64
                + pa[1].abs_diff(pb[1]) as u64
                + pa[2].abs_diff(pb[2]) as u64;
            n += 1;
        }
    }
    n > 0 && (sum / n) <= LIST_UNCHANGED_MEAN_DELTA
}

pub fn crop_row(list: &RgbImage, band: &RowBand) -> Option<RgbImage> {
    let r = band.rect;
    if r.w == 0 || r.h == 0 {
        return None;
    }
    let x = r.x.min(list.width().saturating_sub(1));
    let y = r.y.min(list.height().saturating_sub(1));
    let w = r.w.min(list.width().saturating_sub(x));
    let h = r.h.min(list.height().saturating_sub(y));
    if w == 0 || h == 0 {
        return None;
    }
    Some(list.view(x, y, w, h).to_image())
}

fn luma(p: &Rgb<u8>) -> u8 {
    ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn fill_rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, color: Rgb<u8>) {
        for yy in y..y + h {
            for xx in x..x + w {
                if xx < img.width() && yy < img.height() {
                    img.put_pixel(xx, yy, color);
                }
            }
        }
    }

    #[test]
    fn detects_a_large_bright_panel() {
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb([20, 20, 20]));
        fill_rect(&mut img, 400, 100, 1100, 850, Rgb([220, 220, 220]));
        let rect = detect_list_rect(&img).expect("bright panel");
        assert!(rect.w > 900, "w={}", rect.w);
        assert!(rect.h > 700, "h={}", rect.h);
        assert!(rect.x < 500, "x={}", rect.x);
    }

    #[test]
    fn ignores_screens_without_a_list_panel() {
        let img = RgbImage::from_pixel(1920, 1080, Rgb([20, 20, 20]));
        assert!(detect_list_rect(&img).is_none());
    }

    #[test]
    fn splits_cream_cards_on_hairline_dividers() {
        let mut img = RgbImage::from_pixel(400, 400, Rgb([231, 231, 231]));
        for y in [125u32, 250, 375] {
            fill_rect(&mut img, 0, y, 400, 4, Rgb([206, 206, 206]));
        }
        let rows = split_row_bands(&img, true);
        assert!(
            rows.len() >= 3,
            "expected at least 3 rows, got {}",
            rows.len()
        );
    }

    #[test]
    fn ignores_icon_holes_in_a_cream_panel() {
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb([20, 20, 20]));
        fill_rect(&mut img, 700, 120, 1140, 860, Rgb([231, 231, 231]));
        for y in (140..960).step_by(125) {
            fill_rect(&mut img, 720, y, 80, 50, Rgb([90, 90, 90]));
        }
        let rect = detect_list_rect(&img).expect("gapped cream panel");
        assert!(rect.x >= 680, "x={} should sit on the right-hand list", rect.x);
        assert!(rect.w > 900, "w={}", rect.w);
        assert!(rect.h > 700, "h={}", rect.h);
    }

    #[test]
    fn crop_frac_returns_none_for_empty() {
        let img = RgbImage::from_pixel(10, 10, Rgb([0, 0, 0]));
        assert!(crop_frac(&img, (0.0, 0.0, 0.0, 1.0)).is_none());
    }

    #[test]
    fn detects_the_left_selected_category_row() {
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb([40, 45, 60]));
        fill_rect(&mut img, 70, 176, 310, 96, Rgb([236, 229, 216]));
        fill_rect(&mut img, 700, 120, 1140, 860, Rgb([231, 231, 231]));
        let rect = detect_selected_category_rect(&img).expect("selected row");
        assert!(rect.y >= 160 && rect.y < 230, "y={}", rect.y);
        assert!(rect.h >= 50 && rect.h <= 120, "h={}", rect.h);
        assert!(rect.x < 200, "x={}", rect.x);
        assert!(rect.w > 150, "w={}", rect.w);
    }

    #[test]
    fn list_nearly_equal_accepts_identical_crops() {
        let a = RgbImage::from_pixel(64, 64, Rgb([231, 231, 231]));
        let b = a.clone();
        assert!(list_nearly_equal(&a, &b));
        let mut c = a.clone();
        fill_rect(&mut c, 0, 0, 64, 64, Rgb([10, 10, 10]));
        assert!(!list_nearly_equal(&a, &c));
    }

    #[test]
    fn list_nearly_equal_ignores_status_date_flicker() {
        let a = RgbImage::from_pixel(100, 40, Rgb([231, 231, 231]));
        let mut b = a.clone();
        fill_rect(&mut b, 70, 0, 30, 40, Rgb([40, 40, 40]));
        assert!(list_nearly_equal(&a, &b));
        fill_rect(&mut b, 0, 0, 40, 40, Rgb([10, 10, 10]));
        assert!(!list_nearly_equal(&a, &b));
    }
}
