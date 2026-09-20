//! Pixel-space ports of cocogoat's achievement CV helpers, without OpenCV.
//!
//! `cvGetRect` → [`detect_list_rect`]
//! `cvSplitImage` → [`split_row_bands`]
//! `cvSplitAchievement` → relative field crops in [`crop_frac`]

use image::{GenericImageView, Rgb, RgbImage};

use super::layout::{
    LIST_MIN_HEIGHT_RATIO, LIST_MIN_WIDTH_RATIO, LIST_PANEL_GRAY, PARTIAL_ROW_HEIGHT_RATIO,
    ROW_SEPARATOR_GRAY,
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

/// Detect the large bright achievement list panel in a full-window capture.
///
/// Cocogoat thresholds at 200, finds contours wider than half the image and
/// taller than 75% of it, then insets. Here we take the bounding box of the
/// bright rows in the center band and apply the same inset.
pub fn detect_list_rect(image: &RgbImage) -> Option<PixelRect> {
    let width = image.width();
    let height = image.height();
    if width < 32 || height < 32 {
        return None;
    }

    let mut row_bright = vec![0.0f32; height as usize];
    let x0 = width / 4;
    let x1 = width * 3 / 4;
    let span = (x1 - x0).max(1);
    for y in 0..height {
        let mut count = 0u32;
        for x in x0..x1 {
            if luma(image.get_pixel(x, y)) >= LIST_PANEL_GRAY {
                count += 1;
            }
        }
        row_bright[y as usize] = count as f32 / span as f32;
    }

    let threshold = 0.55;
    let mut best = (0u32, 0u32); // start, len
    let mut run_start = 0u32;
    let mut in_run = false;
    for y in 0..height {
        if row_bright[y as usize] >= threshold {
            if !in_run {
                run_start = y;
                in_run = true;
            }
        } else if in_run {
            let len = y - run_start;
            if len > best.1 {
                best = (run_start, len);
            }
            in_run = false;
        }
    }
    if in_run {
        let len = height - run_start;
        if len > best.1 {
            best = (run_start, len);
        }
    }

    if (best.1 as f32) < height as f32 * LIST_MIN_HEIGHT_RATIO {
        return None;
    }

    let y0 = best.0;
    let y1 = best.0 + best.1;
    let mut min_x = width;
    let mut max_x = 0u32;
    for y in y0..y1 {
        for x in 0..width {
            if luma(image.get_pixel(x, y)) >= LIST_PANEL_GRAY {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
            }
        }
    }
    if max_x <= min_x {
        return None;
    }
    let panel_w = max_x - min_x + 1;
    let panel_h = y1 - y0;
    if (panel_w as f32) < width as f32 * LIST_MIN_WIDTH_RATIO {
        return None;
    }
    // Nearly full-window contours are rejected in cocogoat.
    if panel_w + width / 100 >= width && panel_h + width / 100 >= height {
        return None;
    }

    let inset_x = panel_w / 30;
    let inset_y = panel_h / 100;
    let x = min_x.saturating_add(inset_x);
    let y = y0.saturating_add(inset_y);
    let w = panel_w.saturating_sub(inset_x + inset_x * 3 / 2).max(8);
    let h = panel_h.saturating_sub(inset_y * 2).max(8);
    let w = w.min(width.saturating_sub(x));
    let h = h.min(height.saturating_sub(y));
    Some(PixelRect { x, y, w, h })
}

/// Split a list-panel crop into card rows.
///
/// Cocogoat looks at a right-hand strip, thresholds at 223, morphologically
/// closes bright status/date text into full-width bars, and takes each bar's
/// bottom as a row boundary. We approximate the morphology with a vertical
/// max-filter and treat high-brightness runs as those bars.
pub fn split_row_bands(list: &RgbImage, keep_last: bool) -> Vec<RowBand> {
    let width = list.width();
    let height = list.height();
    if width < 16 || height < 16 {
        return Vec::new();
    }

    let x0 = width - width / 3;
    let strip_w = (width / 4).max(8);
    let mut bright = vec![0.0f32; height as usize];
    for y in 0..height {
        let mut count = 0u32;
        for dx in 0..strip_w {
            let x = (x0 + dx).min(width - 1);
            if luma(list.get_pixel(x, y)) >= ROW_SEPARATOR_GRAY {
                count += 1;
            }
        }
        bright[y as usize] = count as f32 / strip_w as f32;
    }

    // 7px vertical dilation (cocogoat `Mat.ones(7, 7)`).
    let mut dilated = bright.clone();
    let radius = 3i32;
    for y in 0..height as i32 {
        let mut m = 0.0f32;
        for dy in -radius..=radius {
            let yy = (y + dy).clamp(0, height as i32 - 1) as usize;
            m = m.max(bright[yy]);
        }
        dilated[y as usize] = m;
    }

    let mut bottoms: Vec<u32> = Vec::new();
    let mut in_bar = false;
    for y in 0..height {
        if dilated[y as usize] >= 0.55 {
            in_bar = true;
        } else if in_bar {
            bottoms.push(y);
            in_bar = false;
        }
    }
    if in_bar {
        bottoms.push(height);
    }
    if bottoms.is_empty() {
        return Vec::new();
    }

    let mut rows = Vec::new();
    let mut prev = 0u32;
    for bottom in bottoms {
        if bottom <= prev {
            continue;
        }
        rows.push(RowBand {
            rect: PixelRect {
                x: 0,
                y: prev,
                w: width,
                h: bottom - prev,
            },
        });
        prev = bottom;
    }

    if rows.is_empty() {
        return rows;
    }

    let mut heights: Vec<u32> = rows.iter().map(|r| r.rect.h).collect();
    heights.sort_unstable();
    let median = heights[heights.len() / 2] as f32;

    if !keep_last {
        while rows.len() > 1 {
            let last_h = rows.last().unwrap().rect.h as f32;
            if last_h < median * PARTIAL_ROW_HEIGHT_RATIO {
                rows.pop();
            } else {
                break;
            }
        }
        // Cocogoat also drops a last row that sits within 10px of the crop bottom.
        if let Some(last) = rows.last() {
            if height.saturating_sub(last.rect.y + last.rect.h) < 10 && rows.len() > 1 {
                rows.pop();
            }
        }
    }

    rows.retain(|r| r.rect.h >= 16);
    rows
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
    fn splits_bright_status_bands_into_rows() {
        let mut img = RgbImage::from_pixel(400, 300, Rgb([40, 40, 40]));
        // Three cards: bright status strip on the right of each card.
        for (y, h) in [(10u32, 70u32), (100, 70), (190, 70)] {
            fill_rect(&mut img, 0, y, 400, h, Rgb([60, 60, 70]));
            fill_rect(&mut img, 280, y + 8, 90, 24, Rgb([240, 240, 240]));
        }
        let rows = split_row_bands(&img, true);
        assert!(
            rows.len() >= 3,
            "expected at least 3 rows, got {}",
            rows.len()
        );
    }

    #[test]
    fn crop_frac_returns_none_for_empty() {
        let img = RgbImage::from_pixel(10, 10, Rgb([0, 0, 0]));
        assert!(crop_frac(&img, (0.0, 0.0, 0.0, 1.0)).is_none());
    }
}
