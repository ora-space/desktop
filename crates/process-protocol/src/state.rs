use serde::{Deserialize, Serialize};

/// Direct exit evidence is separate from descendant cleanup and business success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExitOutcome {
    Code(i32),
    Signal(i32),
    Unknown,
}

/// A directly launched process can be absent, alive, exited, or not currently verifiable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirectProcessState {
    NotStarted,
    Running,
    Exited(ExitOutcome),
    Unknown,
}

/// Completed cleanup retains the guarantee originally selected for the scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupEvidence {
    ConfirmedQuiescence,
    BestEffortComplete,
}

/// Failure to verify or perform cleanup is observable and never becomes a successful exit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupState {
    Pending,
    Blocked(String),
    Complete(CleanupEvidence),
}

/// Closing seals admission permanently before any cleanup work begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScopeState {
    Open,
    Closing,
    Closed(CleanupEvidence),
}

/// Stop intent is distinct from both signal delivery and confirmed cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopRequest {
    Wait { timeout: std::time::Duration },
    NotifyThenWait { timeout: std::time::Duration },
    Force,
}
