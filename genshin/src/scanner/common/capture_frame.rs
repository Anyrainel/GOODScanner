use anyhow::Result;
use image::{GenericImageView, RgbImage};

use super::coord_scaler::CoordScaler;
use super::game_controller::GenshinGameController;

/// A screenshot crop anchored to base 1920×1080 window coordinates.
///
/// OCR and pixel helpers translate window-space rects into image pixels via
/// [`CaptureFrame::origin`]. Full-window frames use origin `(0, 0)`.
#[derive(Clone, Debug)]
pub struct CaptureFrame {
    pub image: RgbImage,
    pub origin: (f64, f64),
}

impl CaptureFrame {
    pub fn full(image: RgbImage) -> Self {
        Self {
            image,
            origin: (0.0, 0.0),
        }
    }

    pub fn from_region(
        ctrl: &GenshinGameController,
        rect: (f64, f64, f64, f64),
    ) -> Result<Self> {
        let (x, y, w, h) = rect;
        Ok(Self {
            image: ctrl.capture_region(x, y, w, h)?,
            origin: (x, y),
        })
    }

    pub fn is_full_window(&self) -> bool {
        self.origin.0 == 0.0 && self.origin.1 == 0.0
    }

    /// Read a pixel at window base coordinates.
    pub fn pixel(&self, scaler: &CoordScaler, bx: f64, by: f64) -> [u8; 3] {
        let x = (scaler.x(bx) - scaler.x(self.origin.0)) as u32;
        let y = (scaler.y(by) - scaler.y(self.origin.1)) as u32;
        if x < self.image.width() && y < self.image.height() {
            let p = self.image.get_pixel(x, y);
            [p[0], p[1], p[2]]
        } else {
            [0, 0, 0]
        }
    }

    /// Crop a window-space rect from this frame (after optional Y shift).
    pub fn crop(
        &self,
        rect: (f64, f64, f64, f64),
        y_shift: f64,
        scaler: &CoordScaler,
    ) -> Option<RgbImage> {
        let (bx, by, bw, bh) = rect;
        let rel = (
            bx - self.origin.0,
            by + y_shift - self.origin.1,
            bw,
            bh,
        );
        crop_region(&self.image, rel, scaler)
    }
}

/// Crop using coordinates relative to the image origin (already translated).
fn crop_region(
    image: &RgbImage,
    rect: (f64, f64, f64, f64),
    scaler: &CoordScaler,
) -> Option<RgbImage> {
    let (bx, by, bw, bh) = rect;
    let x = (scaler.x(bx) as u32).min(image.width().saturating_sub(1));
    let y = (scaler.y(by) as u32).min(image.height().saturating_sub(1));
    let w = (scaler.x(bw) as u32).min(image.width().saturating_sub(x));
    let h = (scaler.y(bh) as u32).min(image.height().saturating_sub(y));
    if w == 0 || h == 0 {
        return None;
    }
    Some(image.view(x, y, w, h).to_image())
}
