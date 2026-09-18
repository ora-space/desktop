use ora_process_protocol::{ContainmentGuarantee, DirectProcessState, RunId, RunSpec};
use ora_process_protocol::{OutputRead, OutputStream};

/// Optional output capability; does not expose the platform's mutable control authority.
pub trait OutputPlatform: Platform {
    /// Reads a bounded retained range; unknown, uncaptured and not-started runs return an error.
    fn read_output(
        &self,
        run: RunId,
        stream: OutputStream,
        offset: usize,
        max_bytes: usize,
    ) -> Result<OutputRead, PlatformError>;
}

/// An operational failure retains the attempt's cleanup responsibility.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct PlatformError(pub String);

/// Liveness in the adapter's declared containment boundary, including all its tracked descendants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainmentObservation {
    Occupied,
    Empty,
    Unknown,
}

/// Direct exit and containment evidence must be observed independently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformObservation {
    pub direct: DirectProcessState,
    pub containment: ContainmentObservation,
}

/// A process-level action, never a plugin business-protocol command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopSignal {
    RequestExit,
    Force,
}

/// Spawn failures explicitly distinguish proven non-execution from an uncertain side effect.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpawnError {
    /// Proves that neither a workload nor a provisional resource requiring cleanup remains.
    #[error("process was not started: {0}")]
    NotStarted(String),
    /// Retains responsibility whenever execution or provisional-resource cleanup is uncertain.
    #[error("whether the process started is unknown: {0}")]
    Unknown(String),
}

/// Capabilities verified by a platform adapter for its actual deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformCapabilities {
    Unavailable,
    BestEffortOnly,
    StrongOnly,
    StrongAndBestEffort,
}

/// Supplies platform facts to a scope's lifecycle coordinator.
///
/// Implementations must report verified deployment capabilities, not infer them from the OS name.
/// A fake implementation may supply controlled facts for lifecycle tests without claiming OS proof.
pub trait Platform {
    /// Reports guarantees that this adapter can actually enforce before business execution.
    fn capabilities(&self) -> PlatformCapabilities;

    /// Starts one attempt under the frozen guarantee before any business code executes.
    ///
    /// After an uncertain outcome the adapter must retain enough identity to observe and clean up
    /// the attempt. Capability loss must fail this operation, never silently lower its guarantee.
    fn spawn(
        &mut self,
        run: RunId,
        spec: &RunSpec,
        guarantee: ContainmentGuarantee,
    ) -> Result<(), SpawnError>;

    /// Observes the original attempt using stable platform identity rather than a reused PID.
    ///
    /// Empty must satisfy the frozen guarantee. Unknown or failed verification must not report
    /// Empty; an adapter must not create new descendants after reporting completed containment.
    /// Observations must retain terminal direct exit facts rather than report a reused identity.
    fn observe(&mut self, run: RunId) -> Result<PlatformObservation, PlatformError>;

    /// Delivers a stop action to only this run's containment; success does not prove cleanup.
    fn signal(&mut self, run: RunId, signal: StopSignal) -> Result<(), PlatformError>;
}
