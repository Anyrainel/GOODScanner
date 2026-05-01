use crate::capture::Capturer;
use crate::capture::ScreenshotsCapturer;
use crate::capture::WinapiCapturer;
use crate::positioning::Rect;
use anyhow::anyhow;
use anyhow::Result;
use image::RgbImage;

pub struct WindowsCapturer {
    windows_capturer: WinapiCapturer,
    fallback_capturer: ScreenshotsCapturer,
}

impl WindowsCapturer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            windows_capturer: WinapiCapturer::new(),
            fallback_capturer: ScreenshotsCapturer::new()?,
        })
    }
}

impl Capturer<RgbImage> for WindowsCapturer {
    fn capture_rect(&self, rect: Rect<i32>) -> Result<RgbImage> {
        {
            let result = self.windows_capturer.capture_rect(rect);
            if result.is_ok() {
                return result;
            }
        }

        {
            let result = self.fallback_capturer.capture_rect(rect);
            return result;
        }
    }
}
