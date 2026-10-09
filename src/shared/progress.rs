//! Progress reporting (Observer pattern). Features emit events; each UI decides how to show them.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    ScanSource,
    ScanDestination,
    Compare,
    Sync,
}

impl Step {
    pub fn label(self) -> &'static str {
        match self {
            Self::ScanSource => "Scanning source",
            Self::ScanDestination => "Scanning destination",
            Self::Compare => "Comparing",
            Self::Sync => "Syncing",
        }
    }
}

/// Receives progress events. Must be thread-safe: events come from parallel workers.
pub trait ProgressReporter: Send + Sync {
    /// A step started. `total` is `None` when the amount of work is unknown (scanning).
    fn step_started(&self, _step: Step, _total: Option<u64>) {}
    fn advance(&self, _step: Step, _amount: u64) {}
    /// A file failed during the step.
    fn problem(&self, _step: Step) {}
    fn step_finished(&self, _step: Step) {}
}

/// Reporter that ignores every event.
pub struct NoProgress;

impl ProgressReporter for NoProgress {}
