#[cfg(target_os = "windows")]
use crate::capture::WindowsCapturer;
#[cfg(target_os = "windows")]
pub type GenericCapturer = WindowsCapturer;

// On Linux the default backend is the X11 window capturer (works on both X11
// sessions and Wayland sessions via XWayland). The libwayshot / screenshots
// backends stay available as explicit choices via their features; they no
// longer claim the GenericCapturer slot.
#[cfg(target_os = "linux")]
use crate::capture::X11Capturer;
#[cfg(target_os = "linux")]
pub type GenericCapturer = X11Capturer;

// #[cfg(target_os = "macos")]
// pub type GenericCapturer =
