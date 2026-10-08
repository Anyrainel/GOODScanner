use serde::{Deserialize, Serialize};

/// Which Win32 API to use for screen capture.
///
/// BitBlt copies desktop pixels on demand; intended for visible SDR windows.
/// Wgc captures window frames (our HWND path needs Windows 10 1903+).
/// HDR needs FP16 capture and explicit tone mapping in WgcCapturer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMethod {
    #[default]
    BitBlt,
    Wgc,
}

impl CaptureMethod {
    pub fn for_hdr_mode(hdr_mode: bool) -> Self {
        if hdr_mode {
            Self::Wgc
        } else {
            Self::BitBlt
        }
    }
}
