use super::AgentHandle;
use std::sync::atomic::Ordering;

/// Agent-wide work, regardless of which client submitted it. Failure to take
/// a short lock is Unknown (an error), never evidence that the Agent is idle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentActivity {
    pub running: usize,
    pub queued: usize,
    pub approvals: usize,
    pub compactions: usize,
    pub commands: usize,
    pub background: usize,
    pub stopping: bool,
}

impl AgentActivity {
    pub fn is_idle(&self) -> bool {
        self.running
            + self.queued
            + self.approvals
            + self.compactions
            + self.commands
            + self.background
            == 0
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Agent activity is temporarily unavailable")]
pub struct ActivityUnavailable;

impl AgentHandle {
    /// Reserve an idle service for an already prepared update. Admission and
    /// the activity revision are checked under the registration mutex, so a
    /// CLI command arriving after the idle snapshot cannot be silently stopped.
    pub fn close_idle_admission(&self) -> Result<bool, ActivityUnavailable> {
        let revision = self.state.service_work.revision();
        let activity = self.activity_snapshot()?;
        Ok(activity.is_idle() && self.state.service_work.close_if_revision(revision))
    }

    pub fn activity_snapshot(&self) -> Result<AgentActivity, ActivityUnavailable> {
        if self
            .shutdown
            .report()
            .is_some_and(|report| report.status == super::ShutdownStatus::FailedStopping)
        {
            return Err(ActivityUnavailable);
        }
        let revision = self.state.service_work.revision();
        let (commands, background) = self.state.service_work.activity_counts();
        let sessions = self.state.sessions.try_read().ok_or(ActivityUnavailable)?;
        let mut activity = AgentActivity {
            commands,
            background,
            stopping: self.state.service_work.is_closed(),
            approvals: self
                .state
                .approval_gate
                .try_pending_count()
                .ok_or(ActivityUnavailable)?,
            ..Default::default()
        };
        for session in sessions.values() {
            let session = session.try_read().ok_or(ActivityUnavailable)?;
            activity.running += usize::from(
                session.runtime.has_owned_task() || session.runtime.snapshot().is_some(),
            );
            activity.queued += session.scheduler.queued().len();
            activity.compactions +=
                usize::from(session.compaction_in_progress.load(Ordering::Acquire));
        }
        activity.background += self
            .state
            .session_manager
            .journal_compaction_activity()
            .ok_or(ActivityUnavailable)?;
        if revision != self.state.service_work.revision() {
            return Err(ActivityUnavailable);
        }
        Ok(activity)
    }
}
