use image::{imageops, RgbImage};
use sha2::{Digest, Sha256};

use crate::error::{hints, HsrError, HsrResult};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl NormRect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn relative(self, child: NormRect) -> NormRect {
        NormRect {
            x: self.x + child.x * self.width,
            y: self.y + child.y * self.height,
            width: child.width * self.width,
            height: child.height * self.height,
        }
    }

    pub fn center(self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn crop(self, image: &RgbImage) -> HsrResult<RgbImage> {
        if self.x < 0.0
            || self.y < 0.0
            || self.width <= 0.0
            || self.height <= 0.0
            || self.x + self.width > 1.0001
            || self.y + self.height > 1.0001
        {
            return Err(HsrError::new(
                "HSR-VISION-CROP",
                hints::SCREEN_INVALID,
                format!("invalid normalized crop={self:?}"),
            ));
        }
        let x = (self.x * image.width() as f64).floor() as u32;
        let y = (self.y * image.height() as f64).floor() as u32;
        let right = ((self.x + self.width) * image.width() as f64).ceil() as u32;
        let bottom = ((self.y + self.height) * image.height() as f64).ceil() as u32;
        let width = right.min(image.width()).saturating_sub(x);
        let height = bottom.min(image.height()).saturating_sub(y);
        if width == 0 || height == 0 {
            return Err(HsrError::new(
                "HSR-VISION-CROP",
                hints::SCREEN_INVALID,
                "normalized crop produced an empty image",
            ));
        }
        Ok(imageops::crop_imm(image, x, y, width, height).to_image())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GridGeometry {
    pub centers: Vec<Vec<Point>>,
    pub cell_width: f64,
    pub cell_height: f64,
    pub confidence: f64,
}

impl GridGeometry {
    pub fn first(&self) -> Option<Point> {
        self.centers.first().and_then(|row| row.first()).copied()
    }

    pub fn flattened(&self) -> impl Iterator<Item = Point> + '_ {
        self.centers.iter().flatten().copied()
    }

    pub fn rows(&self) -> usize {
        self.centers.len()
    }

    pub fn columns(&self) -> usize {
        self.centers.first().map_or(0, Vec::len)
    }

    pub fn capacity(&self) -> usize {
        self.centers.iter().map(Vec::len).sum()
    }

    pub fn cell(&self, index: usize) -> Option<Point> {
        let columns = self.columns();
        if columns == 0 {
            return None;
        }
        self.centers
            .get(index / columns)
            .and_then(|row| row.get(index % columns))
            .copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollEvidence {
    /// The stable grid and adjacent scroll gutter did not move.
    End { confidence: f64 },
    /// The viewport advanced by this many complete grid rows.
    Advanced { rows: usize, confidence: f64 },
    /// Pixels changed, but they do not prove a row-aligned page advance.
    Uncertain { confidence: f64 },
}

/// Discover the visible HSR inventory grid by fitting a repeated rectangle
/// pattern inside a broad left-side ROI. Legacy 9x5 geometry is used only to
/// bound plausible spacing; the selected origin, stride, row/column counts,
/// and confidence are derived from the current frame.
pub fn discover_inventory_grid(image: &RgbImage) -> HsrResult<GridGeometry> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let luma = luma_plane(image);
    let x_edges = vertical_edge_projection(&luma, width, height, 0.15, 0.86);
    let y_edges = horizontal_edge_projection(&luma, width, height, 0.025, 0.70);

    let x_fit = fit_repeated_axis(&x_edges, (0.035, 0.115), (0.052, 0.078), 7..=10, 0.42)?;
    let y_fit = fit_repeated_axis(&y_edges, (0.18, 0.34), (0.105, 0.18), 3..=6, 0.42)?;

    let mut centers = Vec::new();
    for row in 0..y_fit.count {
        let mut entries = Vec::new();
        for col in 0..x_fit.count {
            entries.push(Point::new(
                (x_fit.start + col as f64 * x_fit.stride) / width as f64,
                (y_fit.start + row as f64 * y_fit.stride) / height as f64,
            ));
        }
        centers.push(entries);
    }

    let confidence = (x_fit.confidence * y_fit.confidence).sqrt();
    if confidence < 0.44 {
        return Err(HsrError::new(
            "HSR-GRID-CONFIDENCE",
            hints::SCREEN_INVALID,
            format!("repeated grid confidence={confidence:.3}; minimum=0.440"),
        ));
    }
    Ok(GridGeometry {
        centers,
        cell_width: x_fit.stride / width as f64 * 0.90,
        cell_height: y_fit.stride / height as f64 * 0.90,
        confidence,
    })
}

#[derive(Debug, Clone, Copy)]
struct AxisFit {
    start: f64,
    stride: f64,
    count: usize,
    confidence: f64,
}

fn fit_repeated_axis(
    edges: &[f64],
    start_range: (f64, f64),
    stride_range: (f64, f64),
    counts: std::ops::RangeInclusive<usize>,
    half_cell_ratio: f64,
) -> HsrResult<AxisFit> {
    let len = edges.len();
    if len < 100 {
        return Err(HsrError::new(
            "HSR-GRID-SIZE",
            hints::SCREEN_INVALID,
            "image axis is too small for grid discovery",
        ));
    }
    let mean = edges.iter().sum::<f64>() / len as f64;
    let mut best: Option<(AxisFit, f64)> = None;
    let start_min = (start_range.0 * len as f64) as usize;
    let start_max = (start_range.1 * len as f64) as usize;
    let stride_min = (stride_range.0 * len as f64) as usize;
    let stride_max = (stride_range.1 * len as f64) as usize;

    // Two-pixel steps keep discovery inexpensive at 4K while still adapting
    // to scale and minor layout drift.
    for start in (start_min..=start_max).step_by(2) {
        for stride in (stride_min..=stride_max).step_by(2) {
            for count in counts.clone() {
                if start + stride * (count.saturating_sub(1)) >= len {
                    continue;
                }
                let half = (stride as f64 * half_cell_ratio).round() as isize;
                let mut score = 0.0;
                for index in 0..count {
                    let center = start + stride * index;
                    score += sample_peak(edges, center as isize - half);
                    score += sample_peak(edges, center as isize + half);
                }
                let normalized = score / (count * 2) as f64;
                // Prefer evidence-rich fits; a tiny count cannot beat a real
                // full-width repeated grid merely because one border is bright.
                let weighted = normalized * (count as f64 / *counts.end() as f64).sqrt();
                if best.is_none_or(|(_, best_score)| weighted > best_score) {
                    let confidence = if mean <= f64::EPSILON {
                        0.0
                    } else {
                        (normalized / (mean * 3.5)).min(1.0)
                    };
                    best = Some((
                        AxisFit {
                            start: start as f64,
                            stride: stride as f64,
                            count,
                            confidence,
                        },
                        weighted,
                    ));
                }
            }
        }
    }
    best.map(|(fit, _)| fit).ok_or_else(|| {
        HsrError::new(
            "HSR-GRID-NOT-FOUND",
            hints::SCREEN_INVALID,
            "no plausible repeated inventory grid was found",
        )
    })
}

fn sample_peak(values: &[f64], index: isize) -> f64 {
    (-2..=2)
        .filter_map(|offset| values.get((index + offset).max(0) as usize))
        .copied()
        .fold(0.0, f64::max)
}

pub fn selected_cell(grid: &GridGeometry, image: &RgbImage) -> Option<(usize, f64)> {
    let mut scores: Vec<(usize, f64)> = grid
        .flattened()
        .enumerate()
        .map(|(index, center)| {
            (
                index,
                ring_brightness(image, center, grid.cell_width, grid.cell_height),
            )
        })
        .collect();
    scores.sort_by(|left, right| right.1.total_cmp(&left.1));
    let best = scores.first().copied()?;
    let second = scores.get(1).map_or(0.0, |entry| entry.1);
    let gap = (best.1 - second).max(0.0);
    (best.1 > 0.25 && gap > 0.015).then_some((best.0, gap))
}

/// Compare two stable inventory screenshots and classify a requested page
/// scroll. The comparison uses card interiors (not the animated selection
/// border) plus a narrow gutter immediately beside the grid. Partial final
/// pages are proven by matching the rows that remain visible after a clamped
/// scroll; a full-page advance is accepted only when all old rows disappear.
pub fn classify_inventory_scroll(
    before: &RgbImage,
    after: &RgbImage,
    grid: &GridGeometry,
    requested_rows: usize,
) -> HsrResult<ScrollEvidence> {
    if before.dimensions() != after.dimensions() {
        return Err(HsrError::new(
            "HSR-SCROLL-GEOMETRY",
            hints::SCREEN_INVALID,
            format!(
                "scroll screenshots have different dimensions; before={:?}, after={:?}",
                before.dimensions(),
                after.dimensions()
            ),
        ));
    }
    let rows = grid.rows();
    let columns = grid.columns();
    if rows == 0 || columns == 0 || requested_rows == 0 || requested_rows > rows {
        return Err(HsrError::new(
            "HSR-SCROLL-REQUEST",
            hints::SCREEN_INVALID,
            format!(
                "invalid scroll request; grid={}x{}, requestedRows={requested_rows}",
                columns, rows
            ),
        ));
    }

    // If the selected card remains visible, its row displacement is strong
    // independent evidence. This also works when adjacent items are genuinely
    // identical and their panel/card pixels cannot distinguish them.
    if requested_rows < rows {
        if let (Some((before_index, before_gap)), Some((after_index, after_gap))) =
            (selected_cell(grid, before), selected_cell(grid, after))
        {
            let before_row = before_index / columns;
            let after_row = after_index / columns;
            if before_gap >= 0.030
                && after_gap >= 0.030
                && before_index % columns == after_index % columns
                && before_row > after_row
            {
                return Ok(ScrollEvidence::Advanced {
                    rows: before_row - after_row,
                    confidence: 1.0,
                });
            }
        }
    }

    let before_rows = grid_row_signatures(before, grid);
    let after_rows = grid_row_signatures(after, grid);
    let unchanged_grid = signature_distance(&before_rows.concat(), &after_rows.concat());
    let before_gutter = scroll_gutter_signature(before, grid);
    let after_gutter = scroll_gutter_signature(after, grid);
    let unchanged_gutter = signature_distance(&before_gutter, &after_gutter);

    if unchanged_grid <= 0.020 && unchanged_gutter <= 0.014 {
        let confidence = 1.0 - (unchanged_grid / 0.020).max(unchanged_gutter / 0.014);
        return Ok(ScrollEvidence::End {
            confidence: confidence.clamp(0.0, 1.0),
        });
    }

    let mut shifts = (1..rows)
        .map(|shift| {
            let distances = (0..rows - shift)
                .map(|after_row| {
                    signature_distance(&before_rows[after_row + shift], &after_rows[after_row])
                })
                .collect::<Vec<_>>();
            let score = distances.iter().sum::<f64>() / distances.len().max(1) as f64;
            (shift, score)
        })
        .collect::<Vec<_>>();
    shifts.sort_by(|left, right| left.1.total_cmp(&right.1));
    if let Some(&(shift, score)) = shifts.first() {
        let separated = shifts.get(1).is_none_or(|second| second.1 - score >= 0.008);
        let maximum_score = if requested_rows == rows { 0.030 } else { 0.080 };
        if score <= maximum_score && (separated || score <= 0.018) {
            return Ok(ScrollEvidence::Advanced {
                rows: shift,
                confidence: (1.0 - score / maximum_score).clamp(0.0, 1.0),
            });
        }
    }

    // A full-page move intentionally has no overlapping row to correlate.
    // Require broad grid change or independent movement in the adjacent
    // scroll gutter; partial-page requests never use this fallback.
    if requested_rows == rows && (unchanged_grid >= 0.065 || unchanged_gutter >= 0.035) {
        let confidence = (unchanged_grid / 0.20)
            .max(unchanged_gutter / 0.12)
            .clamp(0.0, 1.0);
        return Ok(ScrollEvidence::Advanced { rows, confidence });
    }

    Ok(ScrollEvidence::Uncertain {
        confidence: (unchanged_grid.max(unchanged_gutter) / 0.065).clamp(0.0, 1.0),
    })
}

/// Detect a neutral-bright scrollbar thumb at the bottom of the gutter next
/// to the derived inventory grid. An unchanged screenshot alone cannot prove
/// that a wheel event reached the inventory; this independent spatial signal
/// is therefore required before complete inventory coverage is claimed.
pub fn inventory_scrollbar_bottom_confidence(image: &RgbImage, grid: &GridGeometry) -> Option<f64> {
    let (x0, x1, y0, y1) = scroll_gutter_bounds(grid)?;
    let left = (x0 * image.width() as f64).floor() as u32;
    let right = ((x1 * image.width() as f64).ceil() as u32).min(image.width());
    let top = (y0 * image.height() as f64).floor() as u32;
    let bottom = ((y1 * image.height() as f64).ceil() as u32).min(image.height());
    let width = right.saturating_sub(left);
    let height = bottom.saturating_sub(top);
    if width == 0 || height < 4 {
        return None;
    }

    let minimum_bright = (width as usize * 8).div_ceil(100).max(2);
    let active_rows = (top..bottom)
        .map(|y| {
            (left..right)
                .filter(|x| {
                    let [r, g, b] = image.get_pixel(*x, y).0;
                    r >= 160 && g >= 160 && b >= 160 && r.abs_diff(g) <= 35 && g.abs_diff(b) <= 35
                })
                .count()
                >= minimum_bright
        })
        .collect::<Vec<_>>();

    let mut best_start = 0_usize;
    let mut best_end = 0_usize;
    let mut run_start = None;
    for (index, active) in active_rows.iter().copied().chain([false]).enumerate() {
        if active {
            run_start.get_or_insert(index);
        } else if let Some(start) = run_start.take() {
            if index - start > best_end - best_start {
                best_start = start;
                best_end = index;
            }
        }
    }

    let run_length = best_end.saturating_sub(best_start);
    let minimum_run = (height as usize * 2).div_ceil(100).max(3);
    if run_length < minimum_run {
        return None;
    }
    let bottom_gap = active_rows.len().saturating_sub(best_end);
    let bottom_tolerance = (active_rows.len() * 6).div_ceil(100).max(3);
    if bottom_gap > bottom_tolerance {
        return None;
    }
    Some((1.0 - bottom_gap as f64 / bottom_tolerance as f64).clamp(0.0, 1.0))
}

fn grid_row_signatures(image: &RgbImage, grid: &GridGeometry) -> Vec<Vec<u8>> {
    grid.centers
        .iter()
        .map(|row| {
            let mut signature = Vec::with_capacity(row.len() * 36 * 3);
            for center in row {
                // Six-by-six interior samples deliberately omit the card edge,
                // where the selected-card animation lives.
                for sample_y in 0..6 {
                    for sample_x in 0..6 {
                        let dx = (sample_x as f64 + 0.5) / 6.0 - 0.5;
                        let dy = (sample_y as f64 + 0.5) / 6.0 - 0.5;
                        signature.extend_from_slice(&sample_rgb(
                            image,
                            center.x + dx * grid.cell_width * 0.68,
                            center.y + dy * grid.cell_height * 0.68,
                        ));
                    }
                }
            }
            signature
        })
        .collect()
}

fn scroll_gutter_signature(image: &RgbImage, grid: &GridGeometry) -> Vec<u8> {
    let Some((x0, x1, y0, y1)) = scroll_gutter_bounds(grid) else {
        return Vec::new();
    };
    let mut signature = Vec::with_capacity(8 * 48 * 3);
    for sample_y in 0..48 {
        for sample_x in 0..8 {
            signature.extend_from_slice(&sample_rgb(
                image,
                x0 + (sample_x as f64 + 0.5) / 8.0 * (x1 - x0),
                y0 + (sample_y as f64 + 0.5) / 48.0 * (y1 - y0),
            ));
        }
    }
    signature
}

fn scroll_gutter_bounds(grid: &GridGeometry) -> Option<(f64, f64, f64, f64)> {
    let first = grid.first()?;
    let last = grid.centers.last().and_then(|row| row.last()).copied()?;
    let grid_right = grid
        .centers
        .iter()
        .filter_map(|row| row.last())
        .map(|point| point.x)
        .fold(last.x, f64::max)
        + grid.cell_width / 2.0;
    let x0 = (grid_right + 0.006).clamp(0.0, 0.975);
    let x1 = (x0 + 0.040).min(0.995);
    let y0 = (first.y - grid.cell_height / 2.0).clamp(0.0, 0.99);
    let y1 = (last.y + grid.cell_height / 2.0).clamp(y0 + 0.001, 1.0);
    Some((x0, x1, y0, y1))
}

fn sample_rgb(image: &RgbImage, x: f64, y: f64) -> [u8; 3] {
    let px = (x.clamp(0.0, 1.0) * image.width().saturating_sub(1) as f64).round() as u32;
    let py = (y.clamp(0.0, 1.0) * image.height().saturating_sub(1) as f64).round() as u32;
    image.get_pixel(px, py).0
}

fn signature_distance(left: &[u8], right: &[u8]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 1.0;
    }
    left.iter()
        .zip(right)
        .map(|(&a, &b)| a.abs_diff(b) as f64 / 255.0)
        .sum::<f64>()
        / left.len() as f64
}

fn ring_brightness(image: &RgbImage, center: Point, width: f64, height: f64) -> f64 {
    let cx = center.x * image.width() as f64;
    let cy = center.y * image.height() as f64;
    let half_w = width * image.width() as f64 / 2.0;
    let half_h = height * image.height() as f64 / 2.0;
    let mut bright = 0_u64;
    let mut total = 0_u64;
    let thickness = 3.0_f64.max(image.width() as f64 / 640.0);
    let x0 = (cx - half_w).max(0.0) as u32;
    let x1 = (cx + half_w).min(image.width() as f64 - 1.0) as u32;
    let y0 = (cy - half_h).max(0.0) as u32;
    let y1 = (cy + half_h).min(image.height() as f64 - 1.0) as u32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let border = (x as f64 - x0 as f64) < thickness
                || (x1 as f64 - x as f64) < thickness
                || (y as f64 - y0 as f64) < thickness
                || (y1 as f64 - y as f64) < thickness;
            if border {
                let pixel = image.get_pixel(x, y);
                let luma = pixel[0] as u32 + pixel[1] as u32 + pixel[2] as u32;
                bright += u64::from(luma > 570);
                total += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        bright as f64 / total as f64
    }
}

pub fn frame_fingerprint(image: &RgbImage, rect: NormRect) -> HsrResult<String> {
    let crop = rect.crop(image)?;
    let mut hasher = Sha256::new();
    hasher.update(crop.width().to_le_bytes());
    hasher.update(crop.height().to_le_bytes());
    hasher.update(crop.as_raw());
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn frames_similar(left: &RgbImage, right: &RgbImage) -> bool {
    if left.dimensions() != right.dimensions() || left.as_raw().is_empty() {
        return false;
    }
    let mut excess = 0_u64;
    let mut changed = 0_usize;
    for (&a, &b) in left.as_raw().iter().zip(right.as_raw()) {
        let diff = a.abs_diff(b);
        excess += diff.saturating_sub(6) as u64;
        changed += usize::from(diff > 16);
    }
    let len = left.as_raw().len() as f64;
    excess as f64 / len <= 0.20 && changed as f64 / len <= 0.01
}

fn luma_plane(image: &RgbImage) -> Vec<u8> {
    image
        .pixels()
        .map(|pixel| {
            ((pixel[0] as u32 * 54 + pixel[1] as u32 * 183 + pixel[2] as u32 * 19) >> 8) as u8
        })
        .collect()
}

fn vertical_edge_projection(
    luma: &[u8],
    width: usize,
    height: usize,
    y_start: f64,
    y_end: f64,
) -> Vec<f64> {
    let y0 = (height as f64 * y_start) as usize;
    let y1 = (height as f64 * y_end) as usize;
    (0..width)
        .map(|x| {
            if x == 0 {
                return 0.0;
            }
            (y0..y1)
                .map(|y| luma[y * width + x].abs_diff(luma[y * width + x - 1]) as f64)
                .sum::<f64>()
                / (y1 - y0).max(1) as f64
        })
        .collect()
}

fn horizontal_edge_projection(
    luma: &[u8],
    width: usize,
    height: usize,
    x_start: f64,
    x_end: f64,
) -> Vec<f64> {
    let x0 = (width as f64 * x_start) as usize;
    let x1 = (width as f64 * x_end) as usize;
    (0..height)
        .map(|y| {
            if y == 0 {
                return 0.0;
            }
            (x0..x1)
                .map(|x| luma[y * width + x].abs_diff(luma[(y - 1) * width + x]) as f64)
                .sum::<f64>()
                / (x1 - x0).max(1) as f64
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn synthetic_grid(width: u32, height: u32) -> RgbImage {
        let mut image = RgbImage::from_pixel(width, height, Rgb([24, 28, 35]));
        let start_x = (width as f64 * 0.071) as i32;
        let start_y = (height as f64 * 0.26) as i32;
        let stride_x = (width as f64 * 0.0645) as i32;
        let stride_y = (height as f64 * 0.138) as i32;
        let cell_w = (stride_x as f64 * 0.84) as i32;
        let cell_h = (stride_y as f64 * 0.84) as i32;
        for row in 0..5 {
            for col in 0..9 {
                let cx = start_x + col * stride_x;
                let cy = start_y + row * stride_y;
                for x in (cx - cell_w / 2)..=(cx + cell_w / 2) {
                    for y in [cy - cell_h / 2, cy + cell_h / 2] {
                        image.put_pixel(x as u32, y as u32, Rgb([220, 220, 220]));
                    }
                }
                for y in (cy - cell_h / 2)..=(cy + cell_h / 2) {
                    for x in [cx - cell_w / 2, cx + cell_w / 2] {
                        image.put_pixel(x as u32, y as u32, Rgb([220, 220, 220]));
                    }
                }
            }
        }
        image
    }

    fn synthetic_page(
        width: u32,
        height: u32,
        start_row: usize,
        quantity: usize,
        selected_global: Option<usize>,
    ) -> RgbImage {
        let mut image = synthetic_grid(width, height);
        let start_x = (width as f64 * 0.071) as i32;
        let start_y = (height as f64 * 0.26) as i32;
        let stride_x = (width as f64 * 0.0645) as i32;
        let stride_y = (height as f64 * 0.138) as i32;
        let cell_w = (stride_x as f64 * 0.72) as i32;
        let cell_h = (stride_y as f64 * 0.72) as i32;
        for row in 0..5 {
            for col in 0..9 {
                let global = (start_row + row) * 9 + col;
                let cx = start_x + col as i32 * stride_x;
                let cy = start_y + row as i32 * stride_y;
                let color = if global < quantity {
                    Rgb([
                        35 + (global * 29 % 150) as u8,
                        42 + (global * 47 % 140) as u8,
                        50 + (global * 67 % 130) as u8,
                    ])
                } else {
                    Rgb([24, 28, 35])
                };
                for y in (cy - cell_h / 2)..=(cy + cell_h / 2) {
                    for x in (cx - cell_w / 2)..=(cx + cell_w / 2) {
                        image.put_pixel(x as u32, y as u32, color);
                    }
                }
                if selected_global == Some(global) {
                    let select_w = (stride_x as f64 * 0.90) as i32;
                    let select_h = (stride_y as f64 * 0.90) as i32;
                    for thickness in 0..4 {
                        for x in (cx - select_w / 2)..=(cx + select_w / 2) {
                            for y in [cy - select_h / 2 + thickness, cy + select_h / 2 - thickness]
                            {
                                image.put_pixel(x as u32, y as u32, Rgb([250, 250, 250]));
                            }
                        }
                        for y in (cy - select_h / 2)..=(cy + select_h / 2) {
                            for x in [cx - select_w / 2 + thickness, cx + select_w / 2 - thickness]
                            {
                                image.put_pixel(x as u32, y as u32, Rgb([250, 250, 250]));
                            }
                        }
                    }
                }
            }
        }

        // Sanitized synthetic scrollbar thumb. It carries only viewport state,
        // never an account identifier or uncropped game screenshot.
        let thumb_x = (width as f64 * 0.646) as u32;
        let thumb_height = height / 12;
        let total_rows = quantity.div_ceil(9);
        let maximum_start = total_rows.saturating_sub(5);
        let track_bottom = start_y + 4 * stride_y + (stride_y as f64 * 0.90) as i32 / 2;
        let thumb_y = if start_row >= maximum_start {
            (track_bottom as u32).saturating_sub(thumb_height)
        } else {
            (height as f64 * (0.20 + start_row as f64 * 0.018)) as u32
        };
        for y in thumb_y..(thumb_y + thumb_height).min(height) {
            for x in thumb_x..(thumb_x + width / 180).min(width) {
                image.put_pixel(x, y, Rgb([210, 210, 215]));
            }
        }
        image
    }

    #[test]
    fn discovers_scaled_repeated_grid() {
        for (width, height) in [(1280, 720), (1920, 1080), (2560, 1440)] {
            let grid = discover_inventory_grid(&synthetic_grid(width, height)).unwrap();
            assert_eq!(grid.columns(), 9);
            assert_eq!(grid.rows(), 5);
            let first = grid.first().unwrap();
            assert!((first.x - 0.071).abs() < 0.02, "first={first:?}");
            assert!((first.y - 0.26).abs() < 0.03, "first={first:?}");
        }
    }

    #[test]
    fn rejects_blank_screen() {
        let blank = RgbImage::from_pixel(1920, 1080, Rgb([20, 20, 20]));
        assert!(discover_inventory_grid(&blank).is_err());
    }

    #[test]
    fn scrollbar_bottom_requires_a_terminal_thumb_position() {
        let nonterminal = synthetic_page(1280, 720, 0, 100, Some(44));
        let grid = discover_inventory_grid(&nonterminal).unwrap();
        assert_eq!(
            inventory_scrollbar_bottom_confidence(&nonterminal, &grid),
            None
        );

        let terminal = synthetic_page(1280, 720, 1, 50, Some(49));
        assert!(inventory_scrollbar_bottom_confidence(&terminal, &grid)
            .is_some_and(|confidence| confidence >= 0.50));
    }

    #[test]
    fn scroll_classifier_proves_full_partial_and_end_states() {
        let first = synthetic_page(1280, 720, 0, 100, Some(44));
        let grid = discover_inventory_grid(&first).unwrap();
        let full = synthetic_page(1280, 720, 5, 100, Some(44));
        let full_evidence = classify_inventory_scroll(&first, &full, &grid, 5).unwrap();
        assert!(
            matches!(full_evidence, ScrollEvidence::Advanced { rows: 5, .. }),
            "evidence={full_evidence:?}"
        );

        let partial_before = synthetic_page(1280, 720, 0, 50, Some(44));
        let partial_after = synthetic_page(1280, 720, 1, 50, Some(44));
        let partial_grid = discover_inventory_grid(&partial_before).unwrap();
        let partial_evidence =
            classify_inventory_scroll(&partial_before, &partial_after, &partial_grid, 1).unwrap();
        assert!(
            matches!(partial_evidence, ScrollEvidence::Advanced { rows: 1, .. }),
            "evidence={partial_evidence:?}"
        );
        assert!(matches!(
            classify_inventory_scroll(&partial_after, &partial_after, &partial_grid, 1).unwrap(),
            ScrollEvidence::End { .. }
        ));
    }

    #[test]
    fn selection_border_identifies_exact_row_major_cell() {
        let frame = synthetic_page(1920, 1080, 0, 45, Some(17));
        let grid = discover_inventory_grid(&frame).unwrap();
        assert_eq!(selected_cell(&grid, &frame).map(|entry| entry.0), Some(17));
    }
}
