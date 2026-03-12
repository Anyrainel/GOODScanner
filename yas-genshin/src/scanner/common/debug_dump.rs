use std::path::{Path, PathBuf};

use image::{GenericImageView, RgbImage};

use super::coord_scaler::CoordScaler;

/// Context for dumping debug images during scanning.
/// Passed through OCR methods — when `None`, no dumping occurs.
/// When `Some`, images are saved to `base_dir/category/entity_name/field.png`.
pub struct DumpCtx {
    /// Directory for this item, e.g. `debug_images/artifacts/0042_GladiatorsFinale/`
    dir: PathBuf,
}

impl DumpCtx {
    /// Create a new dump context. `entity_name` is used in the folder name
    /// (sanitized for filesystem safety). Creates directories if needed.
    pub fn new(base_dir: &str, category: &str, index: usize, _entity_name: &str) -> Self {
        let folder = format!("{:04}", index);
        let dir = Path::new(base_dir).join(category).join(folder);
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    /// Save the full captured image.
    pub fn dump_full(&self, image: &RgbImage) {
        let path = self.dir.join("full.png");
        let _ = image.save(&path);
    }

    /// Save a cropped OCR region. `rect` is in base 1920x1080 coordinates.
    pub fn dump_region(&self, field_name: &str, image: &RgbImage, rect: (f64, f64, f64, f64), scaler: &CoordScaler) {
        save_region(&self.dir, field_name, image, rect, scaler);
    }

    /// Save a cropped OCR region with Y-shift.
    pub fn dump_region_shifted(&self, field_name: &str, image: &RgbImage, rect: (f64, f64, f64, f64), y_shift: f64, scaler: &CoordScaler) {
        let shifted = (rect.0, rect.1 + y_shift, rect.2, rect.3);
        save_region(&self.dir, field_name, image, shifted, scaler);
    }

    /// Save a pixel-check region: ±padding around center point.
    pub fn dump_pixel(&self, field_name: &str, image: &RgbImage, center: (f64, f64), padding: u32, scaler: &CoordScaler) {
        let cx = scaler.x(center.0) as i32;
        let cy = scaler.y(center.1) as i32;
        let x = (cx - padding as i32).max(0) as u32;
        let y = (cy - padding as i32).max(0) as u32;
        let w = (padding * 2 + 1).min(image.width().saturating_sub(x));
        let h = (padding * 2 + 1).min(image.height().saturating_sub(y));

        if w == 0 || h == 0 {
            return;
        }

        let sub = image.view(x, y, w, h).to_image();
        let path = self.dir.join(format!("{}.png", field_name));
        let _ = sub.save(&path);
    }
}

fn save_region(dir: &Path, name: &str, image: &RgbImage, rect: (f64, f64, f64, f64), scaler: &CoordScaler) {
    let (bx, by, bw, bh) = rect;
    let x = (scaler.x(bx) as u32).min(image.width().saturating_sub(1));
    let y = (scaler.y(by) as u32).min(image.height().saturating_sub(1));
    let w = (scaler.x(bw) as u32).min(image.width().saturating_sub(x));
    let h = (scaler.y(bh) as u32).min(image.height().saturating_sub(y));

    if w == 0 || h == 0 {
        return;
    }

    let sub = image.view(x, y, w, h).to_image();
    let path = dir.join(format!("{}.png", name));
    let _ = sub.save(&path);
}
