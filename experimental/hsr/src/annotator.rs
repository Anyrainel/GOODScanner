//! HSR coordinate adapter over the shared scanner annotation implementation.
use crate::vision::NormRect;
use genshin_scanner::scanner::common::{annotator as shared, coord_scaler::CoordScaler};
use image::RgbImage;

pub fn init(enabled: bool) {
    shared::init_for("hsr", enabled);
}
pub fn is_enabled() -> bool {
    shared::is_enabled()
}
pub fn begin_item(category: &str, index: usize, frame: &RgbImage) {
    shared::begin_item_for(
        "hsr",
        category,
        index,
        &CoordScaler::new(frame.width(), frame.height()),
    );
}
pub fn add_image(label: &str, image: &RgbImage) {
    shared::add_image(label, image);
}
pub fn record_ocr(field: &str, rect: NormRect, raw: &str) {
    shared::record_ocr(
        field,
        (
            rect.x * 1920.0,
            rect.y * 1080.0,
            rect.width * 1920.0,
            rect.height * 1080.0,
        ),
        raw,
    );
}
pub fn set_final(field: &str, result: &str) {
    shared::set_final(field, result);
}
pub fn finalize_success(result: &str) {
    shared::finalize_success(result);
}
pub fn finalize_error(partial: Option<&str>, error: &str) {
    shared::finalize_error(partial, error);
}
pub fn flush() {
    shared::flush();
}

/// One input crop per semantic pixel detection, never one file per sampled pixel.
/// Detection crops are excluded from the OCR training manifest.
pub fn record_detection(field: &str, image: &RgbImage) {
    shared::record_input(field, image, "pixel detection", false);
}

pub fn record_region(field: &str, image: &RgbImage, rect: NormRect) {
    if is_enabled() {
        if let Ok(crop) = rect.crop(image) {
            record_detection(field, &crop);
        }
    }
}

pub fn record_node(field: &str, image: &RgbImage, x: f64, y: f64, radius: f64) {
    let rx = radius * image.height() as f64 / image.width() as f64;
    let left = (x - rx).max(0.0);
    let top = (y - radius).max(0.0);
    record_region(
        field,
        image,
        NormRect::new(
            left,
            top,
            (x + rx).min(1.0) - left,
            (y + radius).min(1.0) - top,
        ),
    );
}
