use crate::game_info::ui::Platform;
use crate::game_info::UI;
use crate::positioning::Rect;

#[derive(Clone, Debug)]
pub struct GameInfo {
    pub window: Rect<i32>,
    pub is_cloud: bool,
    pub ui: UI,
    pub platform: Platform,
    /// Native window handle (HWND on Windows). Used by WGC capturer.
    #[cfg(target_os = "windows")]
    pub hwnd: isize,
    /// X11 window id. Used by the X11 capturer, focus control and liveness
    /// checks (the counterpart of `hwnd` above).
    #[cfg(target_os = "linux")]
    pub window_id: u32,
}
