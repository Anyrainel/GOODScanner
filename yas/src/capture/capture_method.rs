use serde::{Deserialize, Serialize};

/// Which Win32 API to use for screen capture.
///
/// BitBlt   — GDI, works everywhere, but reads from the SDR compositor layer.
///            HDR games (Genshin with Windows HDR enabled) come out clipped-white.
/// PrintWindow — asks the window to render its DirectX content into a GDI DC.
///               Client-area only, no title bar in the output. Handles HDR via
///               the compositor's tone-mapping. Simplest HDR fix; blocked by some
///               anti-cheat configs.
/// Wgc      — Windows.Graphics.Capture (Win10 1803+). Requests
///            B8G8R8A8UIntNormalized: Windows auto-tonemaps HDR→8-bit SDR.
///            Best quality, always works with DX games.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMethod {
    #[default]
    BitBlt,
    PrintWindow,
    Wgc,
}
