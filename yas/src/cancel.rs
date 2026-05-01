//! Per-run cancellation token.
//!
//! Created fresh for each scan or manage invocation. Any code path can
//! check or trigger cancellation. The token carries a typed `StopReason`
//! so callers can produce accurate status messages.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Why a run was stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StopReason {
    /// User pressed right-click or GUI stop button.
    UserAbort = 1,
    /// Game window disappeared mid-run.
    GameLost = 2,
    /// Unrecoverable error during execution.
    Error = 3,
}

impl StopReason {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::UserAbort),
            2 => Some(Self::GameLost),
            3 => Some(Self::Error),
            _ => None,
        }
    }
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserAbort => write!(f, "用户中断 / User aborted"),
            Self::GameLost => write!(f, "游戏窗口丢失 / Game window lost"),
            Self::Error => write!(f, "执行错误 / Execution error"),
        }
    }
}

/// Per-run cancellation token. Cheap to clone (Arc).
#[derive(Clone)]
pub struct CancelToken {
    inner: Arc<AtomicU8>,
}

impl CancelToken {
    /// Create a fresh token (not cancelled).
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicU8::new(0)),
        }
    }

    /// Is this token cancelled for any reason?
    pub fn is_cancelled(&self) -> bool {
        self.inner.load(Ordering::Relaxed) != 0
    }

    /// Get the stop reason, if cancelled.
    pub fn reason(&self) -> Option<StopReason> {
        StopReason::from_u8(self.inner.load(Ordering::Relaxed))
    }

    /// Cancel with a specific reason. First writer wins — subsequent
    /// calls are no-ops (the original reason is preserved).
    pub fn cancel(&self, reason: StopReason) {
        let _ = self
            .inner
            .compare_exchange(0, reason as u8, Ordering::Relaxed, Ordering::Relaxed);
    }

    /// Check the right mouse button and cancel with `UserAbort` if pressed.
    /// Returns `true` if the token is cancelled (for any reason).
    ///
    /// This is the only place that reads the Win32 key state.
    pub fn check_rmb(&self) -> bool {
        if self.is_cancelled() {
            return true;
        }
        if raw_rmb_pressed() {
            self.cancel(StopReason::UserAbort);
            true
        } else {
            false
        }
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
fn raw_rmb_pressed() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_RBUTTON};
    unsafe {
        let state = GetAsyncKeyState(VK_RBUTTON as i32);
        state != 0 && (state & 1) > 0
    }
}

#[cfg(not(windows))]
fn raw_rmb_pressed() -> bool {
    false
}
