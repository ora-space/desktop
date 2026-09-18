use std::collections::BTreeMap;
use std::time::Instant;

use ora_process_protocol::{
    CleanupEvidence, CleanupState, ContainmentGuarantee, ContainmentRequest, DescendantPolicy,
    DirectProcessState, ExitOutcome, LaunchFact, RunId, RunSnapshot, RunSpec, ScopeState,
    StopRequest,
};

use crate::stop::StopPlan;
use crate::{ContainmentObservation, Platform, PlatformCapabilities, PlatformError, SpawnError};

/// A scope cannot be created when its requested guarantee is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionError {
    #[error("the requested containment guarantee is unavailable")]
    Unavailable,
}

/// A repeated identity cannot be used to change the original launch specification.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StartError {
    #[error("run {0} was already associated with a different specification")]
    ConflictingRun(RunId),
    #[error("the scope no longer accepts new runs")]
    ScopeClosed,
}

/// Stop requests cannot implicitly create unknown runs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StopError {
    #[error("unknown run {0}")]
    UnknownRun(RunId),
    #[error("stop deadline cannot be represented")]
    DeadlineOverflow,
}

/// A failed reconciliation is attributable to its run and does not block unrelated runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileFailure {
    pub run: RunId,
    pub error: PlatformError,
}

struct RunRecord {
    spec: RunSpec,
    snapshot: RunSnapshot,
    stop: Option<StopPlan>,
}

/// In-memory guardian coordinator; it does not acknowledge durable or authorized remote requests.
///
/// The host, journal, and platform adapters will own persistence, authorization, and OS evidence.
/// This kernel must not be used as a replacement production spawner before those are integrated.
pub struct ScopeRuntime<P> {
    platform: P,
    guarantee: ContainmentGuarantee,
    runs: BTreeMap<RunId, RunRecord>,
    state: ScopeState,
}

impl<P: crate::OutputPlatform> ScopeRuntime<P> {
    /// Reads volatile output independently of direct exit or completed process cleanup.
    pub fn read_output(
        &self,
        run: RunId,
        stream: ora_process_protocol::OutputStream,
        offset: usize,
        max_bytes: usize,
    ) -> Result<ora_process_protocol::OutputRead, PlatformError> {
        self.platform.read_output(run, stream, offset, max_bytes)
    }
}

impl<P: Platform> ScopeRuntime<P> {
    /// Freezes the actual guarantee before accepting any run into this scope.
    pub fn new(request: ContainmentRequest, platform: P) -> Result<Self, AdmissionError> {
        use ContainmentGuarantee::{BestEffort, Strong};
        use ContainmentRequest::{BestEffort as RequestBestEffort, PreferStrong, RequireStrong};
        use PlatformCapabilities::{BestEffortOnly, StrongAndBestEffort, StrongOnly, Unavailable};

        let guarantee = match (request, platform.capabilities()) {
            (RequireStrong | PreferStrong, StrongOnly | StrongAndBestEffort) => Strong,
            (PreferStrong | RequestBestEffort, BestEffortOnly)
            | (RequestBestEffort, StrongAndBestEffort) => BestEffort,
            (RequireStrong, BestEffortOnly)
            | (RequestBestEffort, StrongOnly)
            | (RequireStrong | PreferStrong | RequestBestEffort, Unavailable) => {
                return Err(AdmissionError::Unavailable);
            }
        };
        Ok(Self {
            platform,
            guarantee,
            runs: BTreeMap::new(),
            state: ScopeState::Open,
        })
    }

    /// Returns the selected guarantee without reinterpreting later platform failures as a downgrade.
    pub fn guarantee(&self) -> ContainmentGuarantee {
        self.guarantee
    }

    /// Recovers an existing attempt or starts a new one without retrying an uncertain spawn.
    pub fn start(&mut self, id: RunId, spec: RunSpec) -> Result<RunSnapshot, StartError> {
        if let Some(record) = self.runs.get(&id) {
            if record.spec != spec {
                return Err(StartError::ConflictingRun(id));
            }
            return Ok(record.snapshot.clone());
        }
        if self.state != ScopeState::Open {
            return Err(StartError::ScopeClosed);
        }
        // Register uncertainty before crossing the side-effect boundary. This is in-memory only;
        // durable acceptance must eventually commit the equivalent intent before calling here.
        let record = self.runs.entry(id).or_insert(RunRecord {
            spec,
            snapshot: RunSnapshot {
                id,
                launch: LaunchFact::Unknown("launch has not been reconciled".into()),
                direct: DirectProcessState::Unknown,
                cleanup: CleanupState::Pending,
            },
            stop: None,
        });
        record.snapshot.launch = match self.platform.spawn(id, &record.spec, self.guarantee) {
            Ok(()) => LaunchFact::Started,
            Err(SpawnError::NotStarted(reason)) => LaunchFact::NotStarted(reason),
            Err(SpawnError::Unknown(reason)) => LaunchFact::Unknown(reason),
        };
        match &record.snapshot.launch {
            LaunchFact::Started => record.snapshot.direct = DirectProcessState::Running,
            LaunchFact::NotStarted(_) => {
                record.snapshot.direct = DirectProcessState::NotStarted;
                record.snapshot.cleanup = CleanupState::Complete(cleanup_evidence(self.guarantee));
            }
            LaunchFact::Unknown(_) => {}
        }
        Ok(record.snapshot.clone())
    }

    /// Queries the original attempt without creating or executing anything for unknown identities.
    pub fn run(&self, id: RunId) -> Option<RunSnapshot> {
        self.runs.get(&id).map(|record| record.snapshot.clone())
    }

    /// Exposes admission and closure separately from each run's direct process result.
    pub fn state(&self) -> ScopeState {
        self.state
    }

    /// Seals admission synchronously; reconciliation retains responsibility after this call returns.
    pub fn close(&mut self, request: StopRequest, now: Instant) -> Result<(), StopError> {
        if matches!(self.state, ScopeState::Closed(_)) {
            return Ok(());
        }
        let plan = StopPlan::new(request, now)?;
        self.state = ScopeState::Closing;
        for record in self.runs.values_mut() {
            if !matches!(record.snapshot.cleanup, CleanupState::Complete(_)) {
                match &mut record.stop {
                    Some(existing) => existing.tighten(&plan),
                    None => record.stop = Some(StopPlan::new(request, now)?),
                }
            }
        }
        Ok(())
    }

    /// Stops only the named attempt without closing its scope or affecting neighboring runs.
    pub fn stop_run(
        &mut self,
        id: RunId,
        request: StopRequest,
        now: Instant,
    ) -> Result<(), StopError> {
        let record = self.runs.get_mut(&id).ok_or(StopError::UnknownRun(id))?;
        if matches!(record.snapshot.cleanup, CleanupState::Complete(_)) {
            return Ok(());
        }
        let plan = StopPlan::new(request, now)?;
        match &mut record.stop {
            Some(existing) => existing.tighten(&plan),
            None => record.stop = Some(plan),
        }
        Ok(())
    }

    /// Advances all accepted responsibilities; operational failures remain visible and retryable.
    pub fn reconcile(&mut self, now: Instant) -> Vec<ReconcileFailure> {
        let evidence = cleanup_evidence(self.guarantee);
        let mut failures = Vec::new();
        for (id, record) in &mut self.runs {
            if matches!(record.snapshot.cleanup, CleanupState::Complete(_)) {
                continue;
            }
            let result = (|| {
                let observation = self.platform.observe(*id)?;
                if record.snapshot.launch == LaunchFact::Started
                    && observation.direct == DirectProcessState::NotStarted
                {
                    return Err(PlatformError(
                        "platform observation contradicts a confirmed launch".into(),
                    ));
                }
                if observation.containment == ContainmentObservation::Empty
                    && observation.direct == DirectProcessState::Running
                {
                    return Err(PlatformError(
                        "empty containment contains a running direct process".into(),
                    ));
                }
                // Terminal evidence is monotonic: missing status may be refined, never retracted.
                record.snapshot.direct = match (record.snapshot.direct, observation.direct) {
                    (
                        previous @ DirectProcessState::Exited(_),
                        DirectProcessState::Unknown
                        | DirectProcessState::Exited(ExitOutcome::Unknown),
                    ) => previous,
                    (
                        DirectProcessState::Exited(ExitOutcome::Unknown),
                        current @ DirectProcessState::Exited(_),
                    ) => current,
                    (previous @ DirectProcessState::Exited(_), current) => {
                        if previous != current {
                            return Err(PlatformError(
                                "platform observation contradicts a confirmed direct exit".into(),
                            ));
                        }
                        previous
                    }
                    (
                        DirectProcessState::NotStarted
                        | DirectProcessState::Running
                        | DirectProcessState::Unknown,
                        current,
                    ) => current,
                };
                record.snapshot.cleanup = match observation.containment {
                    ContainmentObservation::Empty => CleanupState::Complete(evidence),
                    ContainmentObservation::Occupied | ContainmentObservation::Unknown => {
                        CleanupState::Pending
                    }
                };
                if matches!(
                    observation.direct,
                    DirectProcessState::Running | DirectProcessState::Exited(_)
                ) {
                    record.snapshot.launch = LaunchFact::Started;
                }
                if !matches!(record.snapshot.cleanup, CleanupState::Complete(_))
                    && matches!(record.snapshot.direct, DirectProcessState::Exited(_))
                    && let DescendantPolicy::Cleanup { grace } = record.spec.descendants
                {
                    let plan = StopPlan::new(StopRequest::NotifyThenWait { timeout: grace }, now)
                        .map_err(|error| PlatformError(error.to_string()))?;
                    match &mut record.stop {
                        Some(existing) => existing.tighten(&plan),
                        None => record.stop = Some(plan),
                    }
                }
                Ok(())
            })();
            // Observation and delivery can fail independently. Missing evidence must not suppress
            // a force request; successful delivery must still not hide the observation failure.
            let delivery = (|| {
                if !matches!(record.snapshot.cleanup, CleanupState::Complete(_))
                    && let Some(plan) = &mut record.stop
                    && let Some(signal) = plan.pending(now)
                {
                    self.platform.signal(*id, signal)?;
                    plan.delivered(signal);
                }
                Ok(())
            })();
            if let Err(error) = result.and(delivery) {
                record.snapshot.cleanup = CleanupState::Blocked(error.to_string());
                failures.push(ReconcileFailure { run: *id, error });
            }
        }
        if self.state == ScopeState::Closing
            && self
                .runs
                .values()
                .all(|record| matches!(record.snapshot.cleanup, CleanupState::Complete(_)))
        {
            self.state = ScopeState::Closed(evidence);
        }
        failures
    }
}

/// Successful cleanup cannot upgrade the guarantee selected before launch.
fn cleanup_evidence(guarantee: ContainmentGuarantee) -> CleanupEvidence {
    match guarantee {
        ContainmentGuarantee::Strong => CleanupEvidence::ConfirmedQuiescence,
        ContainmentGuarantee::BestEffort => CleanupEvidence::BestEffortComplete,
    }
}
