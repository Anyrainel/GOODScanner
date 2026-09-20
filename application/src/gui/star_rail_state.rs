use std::sync::{Arc, Mutex};

use hsr_scanner::manager::ManagerPlan;

use super::{state::TaskStatus, worker::TaskHandle};

#[derive(Clone)]
pub struct ManagerPreview {
    pub plan: ManagerPlan,
    pub exact_json: String,
    /// True when the exact original plan came from the append-only recovery
    /// journal rather than a new inventory scan.
    pub recovered: bool,
    /// Snapshot of every setting that affects preview/apply. The UI refuses
    /// to apply if any of them changed after the user reviewed the preview.
    pub settings_identity: String,
}

pub struct StarRailState {
    pub data_cache_refresh: super::state::RefreshState,
    pub scan_status: Arc<Mutex<TaskStatus>>,
    pub manager_status: Arc<Mutex<TaskStatus>>,
    pub scan_handle: Option<TaskHandle>,
    pub manager_handle: Option<TaskHandle>,
    pub manager_preview: Arc<Mutex<Option<ManagerPreview>>>,
    pub manager_reviewed: bool,
    pub allow_lock: bool,
    pub allow_unlock: bool,
    pub allow_mark_discard: bool,
    pub allow_unmark_discard: bool,
    #[cfg(feature = "capture")]
    pub capture: super::star_rail_capture_tab::StarRailCaptureState,
}

impl StarRailState {
    pub fn new(output_dir: String) -> Self {
        #[cfg(not(feature = "capture"))]
        let _ = output_dir;
        Self {
            data_cache_refresh: super::state::RefreshState::Idle,
            scan_status: Arc::new(Mutex::new(TaskStatus::Idle)),
            manager_status: Arc::new(Mutex::new(TaskStatus::Idle)),
            scan_handle: None,
            manager_handle: None,
            manager_preview: Arc::new(Mutex::new(None)),
            manager_reviewed: false,
            allow_lock: false,
            allow_unlock: false,
            allow_mark_discard: false,
            allow_unmark_discard: false,
            #[cfg(feature = "capture")]
            capture: super::star_rail_capture_tab::StarRailCaptureState::new(output_dir),
        }
    }

    pub fn scan_running(&self) -> bool {
        self.scan_handle
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
    }

    pub fn manager_running(&self) -> bool {
        self.manager_handle
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
    }

    pub fn is_busy(&self) -> bool {
        let busy = self.scan_running() || self.manager_running();
        #[cfg(feature = "capture")]
        let busy = busy || self.capture.is_busy();
        busy
    }

    pub fn requires_restart(&self) -> bool {
        let required = self
            .scan_handle
            .as_ref()
            .is_some_and(TaskHandle::requires_restart)
            || self
                .manager_handle
                .as_ref()
                .is_some_and(TaskHandle::requires_restart);
        #[cfg(feature = "capture")]
        let required = required || self.capture.requires_restart();
        required
    }

    pub fn invalidate_manager_preview(&mut self) {
        match self.manager_preview.lock() {
            Ok(mut preview) => *preview = None,
            Err(poisoned) => {
                self.manager_preview.clear_poison();
                *poisoned.into_inner() = None;
            },
        }
        self.manager_reviewed = false;
        self.allow_lock = false;
        self.allow_unlock = false;
        self.allow_mark_discard = false;
        self.allow_unmark_discard = false;
    }
}
