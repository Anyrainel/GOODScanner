#[cfg(target_os = "windows")]
use crate::capture::WindowsCapturer;
#[cfg(target_os = "windows")]
pub type GenericCapturer = WindowsCapturer;

#[cfg(all(target_os = "linux", feature = "capturer_libwayshot"))]
use crate::capture::LibwayshotCapturer;
#[cfg(all(target_os = "linux", feature = "capturer_libwayshot"))]
pub type GenericCapturer = LibwayshotCapturer;

#[cfg(all(target_os = "linux", feature = "capturer_screenshots", not(feature = "capturer_libwayshot")))]
use crate::capture::ScreenshotsCapturer;
#[cfg(all(target_os = "linux", feature = "capturer_screenshots", not(feature = "capturer_libwayshot")))]
pub type GenericCapturer = ScreenshotsCapturer;

// #[cfg(target_os = "macos")]
// pub type GenericCapturer = 