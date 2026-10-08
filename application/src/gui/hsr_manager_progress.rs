use super::{
    state::{TaskStatus, UiText},
    task_progress::{Step, StepState, TaskProgress},
};
use hsr_scanner::manager::{
    JournalStatus, ManagerJournal, ManagerJournalStore, ManagerPlan, MutationScope,
};
use std::sync::{Arc, Mutex};

pub fn scope_step(scope: MutationScope) -> Step {
    match scope {
        MutationScope::Lock => Step::new("lock", "锁定", "Lock"),
        MutationScope::Unlock => Step::new("unlock", "解锁", "Unlock"),
        MutationScope::MarkDiscard => Step::new("discard", "标记弃置", "Mark discard"),
        MutationScope::UnmarkDiscard => {
            Step::new("undiscard", "取消弃置标记", "Remove discard marks")
        },
    }
}

pub fn plan_steps(progress: &mut TaskProgress, plan: &ManagerPlan) {
    progress.steps.retain(|s| s.key == "gear");
    for scope in plan.required_scopes() {
        let mut step = scope_step(scope);
        step.total = Some(
            plan.entries
                .iter()
                .flat_map(|e| &e.changes)
                .filter(|c| c.scope == scope)
                .count(),
        );
        progress.steps.push(step);
    }
}

/// Report verified operations only after the existing journal has committed
/// them durably. Lease ownership, recovery, and all errors stay with the store.
pub struct TrackedJournal<S> {
    pub inner: S,
    pub progress: Arc<Mutex<TaskProgress>>,
    pub status: Arc<Mutex<TaskStatus>>,
    pub cancel: yas::cancel::CancelToken,
}

impl<S: ManagerJournalStore> ManagerJournalStore for TrackedJournal<S> {
    fn load(&mut self) -> Result<Option<ManagerJournal>, String> {
        let journal = self.inner.load()?;
        if let Some(journal) = &journal {
            self.update_progress(journal);
        }
        Ok(journal)
    }
    fn holds_exclusive_apply_lease(&self) -> bool {
        self.inner.holds_exclusive_apply_lease()
    }
    fn save(&mut self, journal: &ManagerJournal) -> Result<(), String> {
        self.inner.save(journal)?;
        self.update_progress(journal);
        Ok(())
    }
}

impl<S> TrackedJournal<S> {
    fn update_progress(&self, journal: &ManagerJournal) {
        let mut progress = self.progress.lock().unwrap();
        for scope in journal.plan.required_scopes() {
            let template = scope_step(scope);
            if let Some(step) = progress.steps.iter_mut().find(|s| s.key == template.key) {
                let entries: Vec<_> = journal
                    .entries
                    .iter()
                    .filter(|e| e.change.scope == scope)
                    .collect();
                step.total = Some(entries.len());
                step.completed = entries
                    .iter()
                    .filter(|e| e.status == JournalStatus::Verified)
                    .count();
                step.state = if entries
                    .iter()
                    .any(|e| e.status == JournalStatus::NeedsReview)
                {
                    StepState::Interrupted
                } else if !entries.is_empty() && step.completed == entries.len() {
                    StepState::Complete
                } else if entries.iter().any(|e| e.status != JournalStatus::Pending) {
                    StepState::Running
                } else {
                    StepState::Pending
                };
            }
        }
        if !self.cancel.is_cancelled() {
            if let Some(entry) = journal
                .entries
                .iter()
                .find(|e| e.status == JournalStatus::MutationStarted)
            {
                let step = scope_step(entry.change.scope);
                *self.status.lock().unwrap() = TaskStatus::Running(UiText::new(
                    format!("正在{}", step.zh),
                    format!("Applying: {}", step.en),
                ));
            }
        }
    }
}
