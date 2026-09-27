use crate::capture::Capturer;
use crate::positioning::Rect;
use crate::utils;
use anyhow::Result;
use image::RgbImage;

/// Captures the game window's own X pixmap via GetImage (ZPixmap).
///
/// Unlike a root-screen capture this is independent of monitor layout and of
/// what else is on screen: the X server always has the window's pixels (X11
/// compositors and XWayland keep a backing pixmap per window). Regions are
/// given in root (absolute) coordinates and clipped to the window, matching
/// the semantics of GDI BitBlt on Windows.
pub struct X11Capturer {
    window: u32,
}

impl X11Capturer {
    pub fn new(window: u32) -> Result<Self> {
        // Fail early (bad window id / no X connection) instead of on the
        // first capture deep inside a scan.
        let _ = utils::get_client_rect(window)?;
        Ok(Self { window })
    }
}

impl Capturer<RgbImage> for X11Capturer {
    fn capture_rect(&self, rect: Rect<i32>) -> Result<RgbImage> {
        utils::capture_window_region(self.window, rect)
    }
}
