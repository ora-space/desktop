use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use ora_process_protocol::{
    CleanupState, ContainmentGuarantee, ContainmentRequest, DescendantPolicy, DirectProcessState,
    ExitOutcome, LaunchFact, RunId, RunSnapshot, RunSpec,
};
use ora_process_runtime::{
    ContainmentObservation, Platform, PlatformCapabilities, PlatformError, PlatformObservation,
    ScopeRuntime, SpawnError, StopSignal,
};
use pretty_assertions::assert_eq;

struct ObservedPlatform(Rc<RefCell<PlatformObservation>>);

impl Platform for ObservedPlatform {
    /// Keeps proof of launch independent of the chosen containment strength.
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::BestEffortOnly
    }

    /// Models a lost spawn response whose original execution can later be discovered.
    fn spawn(
        &mut self,
        _run: RunId,
        _spec: &RunSpec,
        _guarantee: ContainmentGuarantee,
    ) -> Result<(), SpawnError> {
        Err(SpawnError::Unknown("spawn response lost".into()))
    }

    /// Supplies externally controlled facts without consulting runtime state.
    fn observe(&mut self, _run: RunId) -> Result<PlatformObservation, PlatformError> {
        Ok(self.0.borrow().clone())
    }

    /// Fact-only scenarios never request a process-level action.
    fn signal(&mut self, _run: RunId, _signal: StopSignal) -> Result<(), PlatformError> {
        Err(PlatformError("unexpected signal".into()))
    }
}

/// specs/test-cases/node/process/lifecycle/attempts-and-closure.md#replaying-a-run-never-creates-a-second-launch-attempt
#[test]
fn discovering_original_process_confirms_launch_without_starting_again() {
    let observation = Rc::new(RefCell::new(PlatformObservation {
        direct: DirectProcessState::Running,
        containment: ContainmentObservation::Occupied,
    }));
    let mut scope = ScopeRuntime::new(
        ContainmentRequest::BestEffort,
        ObservedPlatform(observation),
    )
    .unwrap_or_else(|error| panic!("create scope: {error}"));
    let id = RunId::new();
    let spec = RunSpec::new("git", ".", DescendantPolicy::WaitForAll);
    scope
        .start(id, spec.clone())
        .unwrap_or_else(|error| panic!("start: {error}"));
    assert_eq!(scope.reconcile(Instant::now()), Vec::new());
    let expected = RunSnapshot {
        id,
        launch: LaunchFact::Started,
        direct: DirectProcessState::Running,
        cleanup: CleanupState::Pending,
    };
    assert_eq!(scope.run(id), Some(expected.clone()));
    assert_eq!(scope.start(id, spec), Ok(expected));
}

/// Exit evidence can become more precise, but later uncertainty cannot erase its known status.
#[test]
fn exit_status_is_refined_and_never_erased_by_less_precise_observations() {
    let observation = Rc::new(RefCell::new(PlatformObservation {
        direct: DirectProcessState::Exited(ExitOutcome::Unknown),
        containment: ContainmentObservation::Occupied,
    }));
    let mut scope = ScopeRuntime::new(
        ContainmentRequest::BestEffort,
        ObservedPlatform(observation.clone()),
    )
    .unwrap_or_else(|error| panic!("create scope: {error}"));
    let id = RunId::new();
    scope
        .start(id, RunSpec::new("git", ".", DescendantPolicy::WaitForAll))
        .unwrap_or_else(|error| panic!("start: {error}"));
    let now = Instant::now();
    assert_eq!(scope.reconcile(now), Vec::new());
    observation.borrow_mut().direct = DirectProcessState::Exited(ExitOutcome::Code(7));
    assert_eq!(scope.reconcile(now), Vec::new());
    for direct in [
        DirectProcessState::Exited(ExitOutcome::Unknown),
        DirectProcessState::Unknown,
    ] {
        observation.borrow_mut().direct = direct;
        assert_eq!(scope.reconcile(now), Vec::new());
        assert_eq!(
            scope.run(id),
            Some(RunSnapshot {
                id,
                launch: LaunchFact::Started,
                direct: DirectProcessState::Exited(ExitOutcome::Code(7)),
                cleanup: CleanupState::Pending,
            })
        );
    }
}

/// A later absence claim cannot rewrite an observed execution as a safe non-start.
#[test]
fn observed_launch_cannot_be_rewritten_as_not_started() {
    let observation = Rc::new(RefCell::new(PlatformObservation {
        direct: DirectProcessState::Running,
        containment: ContainmentObservation::Occupied,
    }));
    let mut scope = ScopeRuntime::new(
        ContainmentRequest::BestEffort,
        ObservedPlatform(observation.clone()),
    )
    .unwrap_or_else(|error| panic!("create scope: {error}"));
    let id = RunId::new();
    scope
        .start(id, RunSpec::new("git", ".", DescendantPolicy::WaitForAll))
        .unwrap_or_else(|error| panic!("start: {error}"));
    let now = Instant::now();
    assert_eq!(scope.reconcile(now), Vec::new());
    *observation.borrow_mut() = PlatformObservation {
        direct: DirectProcessState::NotStarted,
        containment: ContainmentObservation::Empty,
    };
    let reason = "platform observation contradicts a confirmed launch";
    assert_eq!(
        scope.reconcile(now),
        vec![ora_process_runtime::ReconcileFailure {
            run: id,
            error: PlatformError(reason.into()),
        }]
    );
    assert_eq!(
        scope.run(id),
        Some(RunSnapshot {
            id,
            launch: LaunchFact::Started,
            direct: DirectProcessState::Running,
            cleanup: CleanupState::Blocked(reason.into()),
        })
    );
}
