//! Pixel helpers for the in-game achievement list.
//!
//! [`detect_list_rect`] finds the right-hand cream card panel.
//! [`split_row_bands`] cuts that panel on the hairline between cards.
//! [`crop_px`] takes title/status crops inside one row.

use image::{GenericImageView, Rgb, RgbImage};

use super::layout::{
    CARD_HEIGHT, CATEGORY_NAME_SHIFT_X, CATEGORY_PROBE_X0, CATEGORY_PROBE_X1, CATEGORY_ROW_H,
    CATEGORY_SELECTED_MAX_H, CATEGORY_SELECTED_MIN_H, LIST_DIVIDER_DARK_RATIO, LIST_MIN_HEIGHT_RATIO,
    LIST_MIN_WIDTH_RATIO, LIST_PANEL_GRAY, LIST_PROBE_X0, LIST_PROBE_X1, LIST_ROW_BRIGHT_RATIO,
    LIST_RUN_GAP, LIST_UNCHANGED_MEAN_DELTA, LIST_X_GAP, MIN_CARD_HEIGHT, MIN_ROW_HEIGHT,
    PARTIAL_ROW_HEIGHT_RATIO, ROW_DIVIDER_MAX, TITLE_BAND, TRAILING_MAX_RATIO,
    TRAILING_PARTIAL_RATIO,
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

    // mid_y can land on a hairline divider (dark), which makes the cream
    // x-span collapse. Use the brightest row inside the panel instead.
    let y1 = y0.saturating_add(panel_h).min(height);
    let span_y = (y0..y1)
        .max_by(|a, b| {
            row_bright[*a as usize]
                .partial_cmp(&row_bright[*b as usize])
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(y0 + panel_h / 2);
    let (min_x, panel_w) = longest_cream_x_span(image, span_y, LIST_X_GAP)?;
    if (panel_w as f32) < width as f32 * LIST_MIN_WIDTH_RATIO {
        return None;
    }

    let inset_x = panel_w / 40;
    let inset_y = panel_h / 80;
    // The selected left-hand category is also cream; never let that bleed in.
    let min_list_x = ((width as f32) * 0.30) as u32;
    let x = min_x.max(min_list_x).saturating_add(inset_x);
    let y = y0.saturating_add(inset_y);
    let right = min_x.saturating_add(panel_w);
    let w = right.saturating_sub(x).saturating_sub(inset_x).max(8);
    let h = panel_h.saturating_sub(inset_y * 2).max(8);
    let w = w.min(width.saturating_sub(x));
    let h = h.min(height.saturating_sub(y));
    Some(PixelRect { x, y, w, h })
}

/// Split a list-panel crop into card rows using the cream-body hairlines.
///
/// Each card is cream (~231). The separator between cards is a *full-width*
/// dip to ~206–213. Sampling a single column false-triggers on subtitle
/// glyphs and slices the title off the card, so we require most of the title
/// column to be dark. Fragments shorter than a real card are then merged.
pub fn split_row_bands(list: &RgbImage, keep_last: bool) -> Vec<RowBand> {
    let width = list.width();
    let height = list.height();
    if width < 16 || height < 16 {
        return Vec::new();
    }

    let mut dividers: Vec<u32> = Vec::new();
    let mut cluster_start: Option<u32> = None;
    for y in 0..height {
        if is_divider_row(list, y) {
            if cluster_start.is_none() {
                cluster_start = Some(y);
            }
        } else if cluster_start.take().is_some() {
            // First cream row after the hairline. Putting the split at the
            // cluster *start* left the hairline on the next card, so TITLE_BAND
            // clipped the first glyphs.
            dividers.push(y);
        }
    }

    let mut rows = Vec::new();
    let mut prev = 0u32;
    for divider in dividers {
        let gap = divider.saturating_sub(prev);
        if gap >= MIN_ROW_HEIGHT {
            rows.push(RowBand {
                rect: PixelRect {
                    x: 0,
                    y: prev,
                    w: width,
                    h: divider - prev,
                },
            });
            prev = divider;
        } else if prev == 0 {
            // Scroll leftover (date line of the previous card) is shorter
            // than MIN_ROW_HEIGHT. Ignoring that hairline used to glue the
            // scrap onto the next card so TITLE_BAND landed in empty cream.
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
    rows = drop_leading_scraps(rows);
    rows = merge_short_bands(rows, width);

    if rows.is_empty() {
        return rows;
    }

    let mut heights: Vec<u32> = rows.iter().map(|r| r.rect.h).collect();
    heights.sort_unstable();
    let median = heights[heights.len() / 2] as f32;

    // Incoming scrap from the previous scroll is a leftover subtitle line.
    while rows.len() > 1 {
        let first_h = rows[0].rect.h as f32;
        if first_h < median * PARTIAL_ROW_HEIGHT_RATIO {
            rows.remove(0);
        } else {
            break;
        }
    }
    // Drop a trailing sliver only. A full last card sits on the panel edge;
    // keep_last used to skip this entirely, which also kept subtitle scraps.
    let _ = keep_last;
    while rows.len() > 1 {
        let last_h = rows.last().unwrap().rect.h as f32;
        if last_h < median * TRAILING_PARTIAL_RATIO || last_h > median * TRAILING_MAX_RATIO {
            rows.pop();
        } else {
            break;
        }
    }

    rows.retain(|r| r.rect.h >= 16);
    rows
}

/// Center of the list's scrollbar thumb. Clicking a card enlarges it; the
/// thumb is the hover target that still receives the wheel.
///
/// The cream panel is full of bright glyphs (primogems, dates). Those are
/// longer than the real thumb, so "longest bright run" clicks a card. The
/// track is a *dark* column with one short pill plus endcaps; pick the pill
/// farthest from both ends of the list.
pub fn detect_scrollbar_thumb(image: &RgbImage, list: PixelRect) -> Option<(u32, u32)> {
    let x0 = list.x.saturating_add(list.w).saturating_sub(8);
    let x1 = list
        .x
        .saturating_add(list.w)
        .saturating_add(48)
        .min(image.width());
    let y0 = list.y;
    let y1 = list.y.saturating_add(list.h).min(image.height());
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    let col_h = y1.saturating_sub(y0).max(1);

    let mut best: Option<(u32, u32, u32)> = None;
    for x in x0..x1 {
        let mut run = 0u32;
        let mut run_start = y0;
        let mut bright_n = 0u32;
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for y in y0..y1 {
            let bright = luma(image.get_pixel(x, y)) >= 220;
            if bright {
                bright_n += 1;
                if run == 0 {
                    run_start = y;
                }
                run += 1;
            }
            if !bright || y + 1 == y1 {
                // A long list has a short thumb (~20px). A short list's thumb
                // is much taller (心跳的记忆 is ~113px) and used to be dropped,
                // which parked the wheel on the cream panel where it does nothing.
                let max_run = (col_h / 2).max(80);
                if run >= 12 && run <= max_run {
                    runs.push((run_start, run));
                }
                run = 0;
            }
        }
        // Cream columns are nearly all bright. A large thumb can be ~15%
        // of the track, so only skip columns that are mostly cream.
        if bright_n.saturating_mul(2) > col_h {
            continue;
        }
        for (run_start, run) in runs {
            let cy = run_start + run / 2;
            let score = cy.saturating_sub(y0).min(y1.saturating_sub(cy));
            if best.map(|(s, _, _)| score > s).unwrap_or(true) {
                best = Some((score, x, cy));
            }
        }
    }
    best.map(|(_, x, y)| (x, y))
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

    let (hit_y0, hit_h) = longest_run_with_gaps(&row_bright, LIST_ROW_BRIGHT_RATIO, 8)?;
    if hit_h < 24 || hit_h > CATEGORY_SELECTED_MAX_H {
        return None;
    }
    // Probe width on the original cream hit. After growing upward the
    // vertical mid can land in the darker name row and miss the x-span.
    let cream_mid_y = hit_y0 + hit_h / 2;
    // A hit on just the "100 %" line is ~40–55px. Grow upward to a full
    // category row so the name is in the crop. This is still a pixel crop
    // of a fixed-size slot, not OCR-then-crop.
    let mut y0 = hit_y0;
    let mut panel_h = hit_h;
    if panel_h < CATEGORY_ROW_H {
        let extra = CATEGORY_ROW_H - panel_h;
        y0 = y0.saturating_sub(extra);
        panel_h = CATEGORY_ROW_H;
    }
    if panel_h < CATEGORY_SELECTED_MIN_H {
        return None;
    }

    let (min_x, panel_w) = longest_cream_x_span_between(
        image,
        cream_mid_y.min(height.saturating_sub(1)),
        x0,
        x1,
        8,
    )?;
    let inset_y = 6u32;
    // Name starts right of the icon. Keep the right edge of the cream row
    // so a long category name is not cut off.
    let x = min_x.saturating_add(CATEGORY_NAME_SHIFT_X);
    let y = y0.saturating_add(inset_y);
    let w = panel_w.saturating_sub(CATEGORY_NAME_SHIFT_X).saturating_sub(2).max(8);
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

/// Crop using x/w as width fractions and y/h as pixels from the card top.
pub fn crop_px(row: &RgbImage, x_frac: f32, y: u32, w_frac: f32, h: u32) -> Option<RgbImage> {
    let x = (row.width() as f32 * x_frac).round() as u32;
    let w = (row.width() as f32 * w_frac).round() as u32;
    if w == 0 || h == 0 || x >= row.width() || y >= row.height() {
        return None;
    }
    let w = w.min(row.width() - x);
    let h = h.min(row.height() - y);
    if w == 0 || h == 0 {
        return None;
    }
    Some(row.view(x, y, w, h).to_image())
}

/// Title crop: find the first dark ink in the title column, then take a
/// fixed-height window. Pixel crop, not OCR-then-crop — title Y moves when
/// the first visible card is top-clipped after a scroll.
pub fn crop_title(row: &RgbImage) -> Option<RgbImage> {
    let y = title_ink_top(row)
        .map(|top| top.saturating_sub(4))
        .unwrap_or(TITLE_BAND.1);
    crop_px(row, TITLE_BAND.0, y, TITLE_BAND.2, TITLE_BAND.3)
}

/// Full card aligned to the bottom of the split row. Extra height on the
/// first card is the banner gap above the title; a short tail is the panel
/// edge and cannot be grown.
pub fn crop_card(row: &RgbImage) -> Option<RgbImage> {
    if row.height() >= CARD_HEIGHT {
        let y = row.height() - CARD_HEIGHT;
        return Some(row.view(0, y, row.width(), CARD_HEIGHT).to_image());
    }
    if row.height() < 40 {
        return None;
    }
    Some(row.clone())
}

fn title_ink_top(row: &RgbImage) -> Option<u32> {
    let width = row.width();
    let height = row.height();
    if width < 16 || height < 16 {
        return None;
    }
    let x0 = ((width as f32) * TITLE_BAND.0).round() as u32;
    // Probe only the left edge of the title. A 2-character title ("罚球", "白船")
    // is under 8% of the full 50% column, so the first row over the threshold
    // was the subtitle and the model read that instead.
    let x1 = (x0 + 160).min(width);
    let y0 = 12u32.min(height.saturating_sub(1));
    let y1 = 80u32.min(height);
    for y in y0..y1 {
        let mut dark = 0u32;
        let mut n = 0u32;
        let mut x = x0;
        while x < x1 {
            if luma(row.get_pixel(x, y)) <= 190 {
                dark += 1;
            }
            n += 1;
            x += 2;
        }
        if n > 0 && (dark as f32 / n as f32) >= 0.08 {
            return Some(y);
        }
    }
    None
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
    let step_x = 8usize;
    let step_y = 8usize;
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

/// True when most of the title column at `y` is a hairline, not glyph noise.
fn is_divider_row(list: &RgbImage, y: u32) -> bool {
    let width = list.width();
    let x0 = (width * 12 / 100).min(width.saturating_sub(1));
    let x1 = (width * 55 / 100).max(x0 + 1);
    let mut dark = 0u32;
    let mut n = 0u32;
    let mut x = x0;
    while x < x1 {
        if luma(list.get_pixel(x, y)) <= ROW_DIVIDER_MAX {
            dark += 1;
        }
        n += 1;
        x += 2;
    }
    n > 0 && (dark as f32 / n as f32) >= LIST_DIVIDER_DARK_RATIO
}

/// Drop a leftover subtitle line so merge cannot glue it onto the next card.
fn drop_leading_scraps(mut rows: Vec<RowBand>) -> Vec<RowBand> {
    while rows.len() > 1 && rows[0].rect.h < MIN_CARD_HEIGHT {
        rows.remove(0);
    }
    rows
}

/// Glue leftover title/subtitle fragments into card-height bands.
fn merge_short_bands(rows: Vec<RowBand>, width: u32) -> Vec<RowBand> {
    let mut merged: Vec<RowBand> = Vec::new();
    for row in rows {
        if let Some(prev) = merged.last_mut() {
            if prev.rect.h < MIN_CARD_HEIGHT {
                let bottom = row.rect.y.saturating_add(row.rect.h);
                prev.rect.h = bottom.saturating_sub(prev.rect.y);
                prev.rect.w = width;
                continue;
            }
        }
        merged.push(row);
    }
    merged
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
        assert!(rect.w > 800, "w={}", rect.w);
        assert!(rect.h > 700, "h={}", rect.h);
        assert!(rect.x >= 500, "x={} should sit on the right-hand list", rect.x);
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
        assert!(
            rows[1].rect.y >= 128,
            "second card y={} should start after the 4px hairline",
            rows[1].rect.y
        );
    }

    #[test]
    fn keeps_a_full_last_card_on_the_panel_edge() {
        let mut img = RgbImage::from_pixel(400, 379, Rgb([231, 231, 231]));
        for y in [125u32, 250] {
            fill_rect(&mut img, 0, y, 400, 4, Rgb([206, 206, 206]));
        }
        let rows = split_row_bands(&img, false);
        assert!(
            rows.len() >= 3,
            "last full card was dropped: heights={:?}",
            rows.iter().map(|r| r.rect.h).collect::<Vec<_>>()
        );
        assert!(
            rows.last().unwrap().rect.h >= 100,
            "last card h={}",
            rows.last().unwrap().rect.h
        );
    }

    #[test]
    fn drops_a_leading_scroll_scrap_instead_of_gluing_it() {
        let mut img = RgbImage::from_pixel(400, 400, Rgb([231, 231, 231]));
        fill_rect(&mut img, 0, 28, 400, 4, Rgb([206, 206, 206]));
        fill_rect(&mut img, 0, 157, 400, 4, Rgb([206, 206, 206]));
        fill_rect(&mut img, 0, 286, 400, 4, Rgb([206, 206, 206]));
        let rows = split_row_bands(&img, true);
        assert!(
            rows[0].rect.y >= 24,
            "first card y={} should skip the date leftover",
            rows[0].rect.y
        );
        assert!(
            rows[0].rect.h >= 110 && rows[0].rect.h <= 140,
            "first card h={} should be one card, not scrap+card",
            rows[0].rect.h
        );
    }

    #[test]
    fn keeps_a_100px_last_card_that_still_has_the_title() {
        let mut img = RgbImage::from_pixel(400, 350, Rgb([231, 231, 231]));
        for y in [125u32, 250] {
            fill_rect(&mut img, 0, y, 400, 4, Rgb([206, 206, 206]));
        }
        let rows = split_row_bands(&img, true);
        let last_h = rows.last().unwrap().rect.h;
        assert!(
            last_h >= 90,
            "100px last card was dropped: heights={:?}",
            rows.iter().map(|r| r.rect.h).collect::<Vec<_>>()
        );
    }

    #[test]
    fn drops_tall_empty_cream_below_the_last_card() {
        let mut img = RgbImage::from_pixel(400, 500, Rgb([231, 231, 231]));
        for y in [125u32, 250] {
            fill_rect(&mut img, 0, y, 400, 4, Rgb([206, 206, 206]));
        }
        let rows = split_row_bands(&img, true);
        assert!(
            rows.last().unwrap().rect.h < 180,
            "empty floor was kept: heights={:?}",
            rows.iter().map(|r| r.rect.h).collect::<Vec<_>>()
        );
    }

    #[test]
    fn subtitle_glyphs_do_not_split_a_card() {
        let mut img = RgbImage::from_pixel(400, 250, Rgb([231, 231, 231]));
        fill_rect(&mut img, 0, 125, 400, 4, Rgb([206, 206, 206]));
        for x in (40..220).step_by(8) {
            img.put_pixel(x, 70, Rgb([80, 80, 80]));
        }
        let rows = split_row_bands(&img, true);
        assert_eq!(
            rows.len(),
            2,
            "got heights {:?}",
            rows.iter().map(|r| r.rect.h).collect::<Vec<_>>()
        );
        assert!(rows[0].rect.h >= 120, "first card h={}", rows[0].rect.h);
    }

    #[test]
    fn merges_title_and_subtitle_fragments() {
        let mut img = RgbImage::from_pixel(400, 250, Rgb([231, 231, 231]));
        fill_rect(&mut img, 0, 70, 400, 4, Rgb([206, 206, 206]));
        fill_rect(&mut img, 0, 125, 400, 4, Rgb([206, 206, 206]));
        let rows = split_row_bands(&img, true);
        assert!(
            rows[0].rect.h >= 120,
            "merged first card h={} rows={:?}",
            rows[0].rect.h,
            rows.iter().map(|r| r.rect.h).collect::<Vec<_>>()
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
        // Sidebar cream runs to ~0.35 of the width, just left of the list.
        fill_rect(&mut img, 70, 176, 600, 96, Rgb([236, 229, 216]));
        fill_rect(&mut img, 700, 120, 1140, 860, Rgb([231, 231, 231]));
        let rect = detect_selected_category_rect(&img).expect("selected row");
        assert!(rect.y >= 160 && rect.y < 230, "y={}", rect.y);
        assert!(rect.h >= 50 && rect.h <= 120, "h={}", rect.h);
        assert!(rect.x < 200, "x={}", rect.x);
        assert!(rect.w > 150, "w={}", rect.w);
    }

    #[test]
    fn expands_a_short_percentage_band_up_to_the_category_name() {
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb([40, 45, 60]));
        // Name row (darker text area) plus a short bright "100 %" strip.
        fill_rect(&mut img, 70, 176, 310, 96, Rgb([200, 195, 185]));
        fill_rect(&mut img, 70, 220, 310, 48, Rgb([236, 229, 216]));
        fill_rect(&mut img, 700, 120, 1140, 860, Rgb([231, 231, 231]));
        let rect = detect_selected_category_rect(&img).expect("expanded row");
        assert!(
            rect.y <= 186,
            "y={} should include the name above the % line",
            rect.y
        );
        assert!(rect.h >= 80, "h={} should be a full category row", rect.h);
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

    #[test]
    fn detect_list_survives_a_hairline_through_mid_y() {
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb([20, 20, 20]));
        fill_rect(&mut img, 700, 100, 1140, 860, Rgb([231, 231, 231]));
        let mid = 100 + 860 / 2;
        fill_rect(&mut img, 700, mid, 1140, 6, Rgb([40, 40, 40]));
        let rect = detect_list_rect(&img).expect("panel with mid hairline");
        assert!(rect.w > 900, "w={}", rect.w);
        assert!(rect.x >= 500, "x={}", rect.x);
    }

    #[test]
    fn finds_a_short_bright_scrollbar_thumb() {
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb([20, 20, 20]));
        fill_rect(&mut img, 700, 120, 1140, 860, Rgb([231, 231, 231]));
        // Primogem / date glyphs inside the cream panel — longer than the thumb.
        fill_rect(&mut img, 1808, 300, 8, 50, Rgb([246, 246, 246]));
        fill_rect(&mut img, 1848, 150, 4, 24, Rgb([246, 246, 246]));
        let list = detect_list_rect(&img).expect("panel");
        let (x, y) = detect_scrollbar_thumb(&img, list).expect("thumb");
        assert!(x >= 1846 && x <= 1852, "x={x}");
        assert!(y >= 150 && y <= 174, "y={y}");
    }
}
