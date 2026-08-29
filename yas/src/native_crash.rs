//! Optional thread-context propagation for an application-owned native crash
//! boundary.
//!
//! The core crates do not install a Windows exception handler themselves.
//! When the GUI installs one, it registers tiny enter/restore hooks here so
//! scanner-created Rust/Rayon/Tokio workers can inherit the active task's
//! recorder without depending on the application crate.

use std::sync::OnceLock;

type EnterHook = fn() -> usize;
type RestoreHook = fn(usize);

static THREAD_HOOKS: OnceLock<(EnterHook, RestoreHook)> = OnceLock::new();

/// Register the GUI's thread-context hooks. The first registration wins.
pub fn install_thread_hooks(enter: EnterHook, restore: RestoreHook) {
    let _ = THREAD_HOOKS.set((enter, restore));
}

/// Guard that restores the thread's previous crash context on drop.
pub struct ThreadContextGuard {
    previous: usize,
    restore: Option<RestoreHook>,
}

impl Drop for ThreadContextGuard {
    fn drop(&mut self) {
        if let Some(restore) = self.restore {
            restore(self.previous);
        }
    }
}

/// Inherit the currently active GUI task's native-crash context on this
/// thread. This is a no-op in CLI/tests where no GUI hook was installed.
pub fn inherit_current_task() -> ThreadContextGuard {
    match THREAD_HOOKS.get().copied() {
        Some((enter, restore)) => ThreadContextGuard {
            previous: enter(),
            restore: Some(restore),
        },
        None => ThreadContextGuard {
            previous: 0,
            restore: None,
        },
    }
}
