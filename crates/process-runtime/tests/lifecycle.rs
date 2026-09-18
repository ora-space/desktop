use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ora_process_protocol::{
    CleanupEvidence, CleanupState, ContainmentGuarantee, ContainmentRequest, DescendantPolicy,
    DirectProcessState, ExitOutcome, RunId, RunSpec, ScopeState, StopRequest,
};
use ora_process_runtime::{
    ContainmentObservation, Platform, PlatformCapabilities, PlatformError, PlatformObservation,
    ScopeRuntime, SpawnError, StartError, StopSignal,
};
use pretty_assertions::assert_eq;

#[derive(Clone, Default)]
struct Machine(Rc<RefCell<BTreeMap<RunId, Process>>>);

struct Process {
    observation: PlatformObservation,
    signals: Vec<StopSignal>,
    signal_error: Option<PlatformError>,
    observation_error: Option<PlatformError>,
}

/// Missing liveness information cannot prevent an already accepted force-stop action.
#[test]
fn unavailable_observation_does_not_prevent_force_stop() {
    let machine = Machine::default();
    let mut scope = ScopeRuntime::new(ContainmentRequest::BestEffort, machine.clone())
        .unwrap_or_else(|error| panic!("create scope: {error}"));
    let id = RunId::new();
    scope
        .start(id, RunSpec::new("git", ".", DescendantPolicy::WaitForAll))
        .unwrap_or_else(|error| panic!("start: {error}"));
    machine
        .0
        .borrow_mut()
        .get_mut(&id)
        .unwrap_or_else(|| panic!("missing {id}"))
        .observation_error = Some(PlatformError("observation unavailable".into()));
    let now = Instant::now();
    assert_eq!(scope.close(StopRequest::Force, now), Ok(()));
    assert_eq!(
        scope.reconcile(now),
        vec![ora_process_runtime::ReconcileFailure {
            run: id,
            error: PlatformError("observation unavailable".into()),
        }]
    );
    assert_eq!(
        machine.0.borrow()[&id].signals.clone(),
        vec![StopSignal::Force]
    );
    assert_eq!(scope.state(), ScopeState::Closing);
}

/// Contradictory adapter facts must not retract an exit or fabricate successful cleanup.
#[test]
fn contradictory_observations_remain_blocked_until_valid_evidence_arrives() {
    let machine = Machine::default();
    let mut scope = ScopeRuntime::new(ContainmentRequest::BestEffort, machine.clone())
        .unwrap_or_else(|error| panic!("create scope: {error}"));
    let id = RunId::new();
    scope
        .start(id, RunSpec::new("git", ".", DescendantPolicy::WaitForAll))
        .unwrap_or_else(|error| panic!("start: {error}"));
    let now = Instant::now();
    machine
        .0
        .borrow_mut()
        .get_mut(&id)
        .unwrap_or_else(|| panic!("missing {id}"))
        .observation
        .direct = DirectProcessState::Exited(ExitOutcome::Code(3));
    assert_eq!(scope.reconcile(now), Vec::new());
    machine
        .0
        .borrow_mut()
        .get_mut(&id)
        .unwrap_or_else(|| panic!("missing {id}"))
        .observation
        .direct = DirectProcessState::Running;
    let reason = "platform observation contradicts a confirmed direct exit";
    assert_eq!(
        scope.reconcile(now),
        vec![ora_process_runtime::ReconcileFailure {
            run: id,
            error: PlatformError(reason.into()),
        }]
    );
    assert_eq!(
        scope.run(id),
        Some(ora_process_protocol::RunSnapshot {
            id,
            launch: ora_process_protocol::LaunchFact::Started,
            direct: DirectProcessState::Exited(ExitOutcome::Code(3)),
            cleanup: CleanupState::Blocked(reason.into()),
        })
    );
    machine
        .0
        .borrow_mut()
        .get_mut(&id)
        .unwrap_or_else(|| panic!("missing {id}"))
        .observation = PlatformObservation {
        direct: DirectProcessState::Exited(ExitOutcome::Code(3)),
        containment: ContainmentObservation::Empty,
    };
    assert_eq!(scope.reconcile(now), Vec::new());
    assert_eq!(
        scope.run(id),
        Some(ora_process_protocol::RunSnapshot {
            id,
            launch: ora_process_protocol::LaunchFact::Started,
            direct: DirectProcessState::Exited(ExitOutcome::Code(3)),
            cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete),
        })
    );
}

/// A failed signal retains responsibility, and temporary uncertainty cannot erase a known exit.
#[test]
fn cleanup_failure_recovers_without_erasing_exit_evidence() {
    let machine = Machine::default();
    let mut scope = ScopeRuntime::new(ContainmentRequest::BestEffort, machine.clone())
        .unwrap_or_else(|error| panic!("create scope: {error}"));
    let id = RunId::new();
    scope
        .start(id, RunSpec::new("git", ".", DescendantPolicy::WaitForAll))
        .unwrap_or_else(|error| panic!("start: {error}"));
    {
        let mut processes = machine.0.borrow_mut();
        let process = processes
            .get_mut(&id)
            .unwrap_or_else(|| panic!("missing {id}"));
        process.observation.direct = DirectProcessState::Exited(ExitOutcome::Code(7));
        process.signal_error = Some(PlatformError("permission denied".into()));
    }
    let now = Instant::now();
    assert_eq!(scope.close(StopRequest::Force, now), Ok(()));
    assert_eq!(
        scope.reconcile(now),
        vec![ora_process_runtime::ReconcileFailure {
            run: id,
            error: PlatformError("permission denied".into()),
        }]
    );
    assert_eq!(
        scope.run(id),
        Some(ora_process_protocol::RunSnapshot {
            id,
            launch: ora_process_protocol::LaunchFact::Started,
            direct: DirectProcessState::Exited(ExitOutcome::Code(7)),
            cleanup: CleanupState::Blocked("permission denied".into()),
        })
    );
    {
        let mut processes = machine.0.borrow_mut();
        let process = processes
            .get_mut(&id)
            .unwrap_or_else(|| panic!("missing {id}"));
        process.observation.direct = DirectProcessState::Unknown;
        process.signal_error = None;
    }
    assert_eq!(scope.reconcile(now), Vec::new());
    assert_eq!(scope.state(), ScopeState::Closing);
    assert_eq!(
        machine.0.borrow()[&id].signals.clone(),
        vec![StopSignal::Force]
    );
    machine
        .0
        .borrow_mut()
        .get_mut(&id)
        .unwrap_or_else(|| panic!("missing {id}"))
        .observation
        .containment = ContainmentObservation::Empty;
    assert_eq!(scope.reconcile(now), Vec::new());
    assert_eq!(
        scope.run(id),
        Some(ora_process_protocol::RunSnapshot {
            id,
            launch: ora_process_protocol::LaunchFact::Started,
            direct: DirectProcessState::Exited(ExitOutcome::Code(7)),
            cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete),
        })
    );
    assert_eq!(
        scope.state(),
        ScopeState::Closed(CleanupEvidence::BestEffortComplete)
    );
}

/// Direct exit preserves its result while the chosen policy governs surviving descendants.
#[test]
fn direct_exit_cleans_descendants_only_under_the_cleanup_policy() {
    let machine = Machine::default();
    let mut scope = ScopeRuntime::new(ContainmentRequest::BestEffort, machine.clone())
        .unwrap_or_else(|error| panic!("create scope: {error}"));
    let cleanup = RunId::new();
    let wait = RunId::new();
    for (id, policy) in [
        (
            cleanup,
            DescendantPolicy::Cleanup {
                grace: Duration::from_secs(/*secs*/ 5),
            },
        ),
        (wait, DescendantPolicy::WaitForAll),
    ] {
        scope
            .start(id, RunSpec::new("git", ".", policy))
            .unwrap_or_else(|error| panic!("start: {error}"));
        machine
            .0
            .borrow_mut()
            .get_mut(&id)
            .unwrap_or_else(|| panic!("missing simulated process {id}"))
            .observation
            .direct = DirectProcessState::Exited(ExitOutcome::Code(0));
    }
    let now = Instant::now();
    assert_eq!(scope.reconcile(now), Vec::new());
    assert_eq!(
        machine.0.borrow()[&cleanup].signals.clone(),
        vec![StopSignal::RequestExit]
    );
    assert_eq!(
        scope.reconcile(now + Duration::from_secs(/*secs*/ 4)),
        Vec::new()
    );
    assert_eq!(
        scope.reconcile(now + Duration::from_secs(/*secs*/ 5)),
        Vec::new()
    );
    assert_eq!(
        machine.0.borrow()[&cleanup].signals.clone(),
        vec![StopSignal::RequestExit, StopSignal::Force]
    );
    assert_eq!(
        machine.0.borrow()[&wait].signals.clone(),
        Vec::<StopSignal>::new()
    );
    for id in [cleanup, wait] {
        assert_eq!(
            scope.run(id),
            Some(ora_process_protocol::RunSnapshot {
                id,
                launch: ora_process_protocol::LaunchFact::Started,
                direct: DirectProcessState::Exited(ExitOutcome::Code(0)),
                cleanup: CleanupState::Pending,
            })
        );
    }
}

/// Repeated requests tighten deadlines without widening the affected run boundary.
#[test]
fn stopping_one_run_never_extends_its_deadline_or_stops_its_neighbor() {
    let machine = Machine::default();
    let mut scope = ScopeRuntime::new(ContainmentRequest::BestEffort, machine.clone())
        .unwrap_or_else(|error| panic!("create scope: {error}"));
    let a = RunId::new();
    let b = RunId::new();
    for id in [a, b] {
        scope
            .start(id, RunSpec::new("git", ".", DescendantPolicy::WaitForAll))
            .unwrap_or_else(|error| panic!("start: {error}"));
    }
    let now = Instant::now();
    assert_eq!(
        scope.stop_run(
            a,
            StopRequest::Wait {
                timeout: Duration::from_secs(/*secs*/ 10)
            },
            now
        ),
        Ok(())
    );
    assert_eq!(scope.reconcile(now), Vec::new());
    assert_eq!(
        machine.0.borrow()[&a].signals.clone(),
        Vec::<StopSignal>::new()
    );
    assert_eq!(
        scope.stop_run(
            a,
            StopRequest::NotifyThenWait {
                timeout: Duration::from_secs(/*secs*/ 20)
            },
            now
        ),
        Ok(())
    );
    assert_eq!(scope.reconcile(now), Vec::new());
    assert_eq!(
        machine.0.borrow()[&a].signals.clone(),
        vec![StopSignal::RequestExit]
    );
    assert_eq!(
        scope.reconcile(now + Duration::from_secs(/*secs*/ 10)),
        Vec::new()
    );
    assert_eq!(
        scope.reconcile(now + Duration::from_secs(/*secs*/ 30)),
        Vec::new()
    );
    assert_eq!(
        machine.0.borrow()[&a].signals.clone(),
        vec![StopSignal::RequestExit, StopSignal::Force]
    );
    assert_eq!(
        machine.0.borrow()[&b].signals.clone(),
        Vec::<StopSignal>::new()
    );
    assert_eq!(scope.state(), ScopeState::Open);
}

impl Platform for Machine {
    /// Supplies an explicitly best-effort platform so cleanup cannot become strong by accident.
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::BestEffortOnly
    }

    /// Models external processes independently of the runtime's own run records.
    fn spawn(
        &mut self,
        run: RunId,
        _spec: &RunSpec,
        _guarantee: ContainmentGuarantee,
    ) -> Result<(), SpawnError> {
        self.0.borrow_mut().insert(
            run,
            Process {
                observation: PlatformObservation {
                    direct: DirectProcessState::Running,
                    containment: ContainmentObservation::Occupied,
                },
                signals: Vec::new(),
                signal_error: None,
                observation_error: None,
            },
        );
        Ok(())
    }

    /// Signals do not change observed liveness: successful delivery is not exit evidence.
    fn signal(&mut self, run: RunId, signal: StopSignal) -> Result<(), PlatformError> {
        let mut processes = self.0.borrow_mut();
        let process = processes
            .get_mut(&run)
            .unwrap_or_else(|| panic!("missing simulated process {run}"));
        if let Some(error) = &process.signal_error {
            return Err(error.clone());
        }
        process.signals.push(signal);
        Ok(())
    }

    /// Returns controlled OS facts through the approved platform seam.
    fn observe(&mut self, run: RunId) -> Result<PlatformObservation, PlatformError> {
        let processes = self.0.borrow();
        let process = processes
            .get(&run)
            .unwrap_or_else(|| panic!("missing simulated process {run}"));
        match &process.observation_error {
            Some(error) => Err(error.clone()),
            None => Ok(process.observation.clone()),
        }
    }
}

/// specs/test-cases/node/process/lifecycle/attempts-and-closure.md#scope-closure-cannot-lose-a-concurrently-accepted-run
#[test]
fn closure_seals_launches_and_waits_for_cleanup_evidence() {
    let machine = Machine::default();
    let mut scope = ScopeRuntime::new(ContainmentRequest::PreferStrong, machine.clone())
        .unwrap_or_else(|error| panic!("create scope: {error}"));
    let id = RunId::new();
    let spec = RunSpec::new("git", ".", DescendantPolicy::WaitForAll);
    let started = scope
        .start(id, spec.clone())
        .unwrap_or_else(|error| panic!("start: {error}"));
    let now = Instant::now();

    assert_eq!(scope.close(StopRequest::Force, now), Ok(()));
    assert_eq!(
        scope.start(RunId::new(), spec.clone()),
        Err(StartError::ScopeClosed)
    );
    assert_eq!(scope.start(id, spec), Ok(started));
    assert_eq!(scope.reconcile(now), Vec::new());
    assert_eq!(scope.state(), ScopeState::Closing);

    machine
        .0
        .borrow_mut()
        .get_mut(&id)
        .unwrap_or_else(|| panic!("missing simulated process {id}"))
        .observation = PlatformObservation {
        direct: DirectProcessState::Exited(ExitOutcome::Code(0)),
        containment: ContainmentObservation::Empty,
    };
    assert_eq!(scope.reconcile(now), Vec::new());
    assert_eq!(
        scope.state(),
        ScopeState::Closed(CleanupEvidence::BestEffortComplete)
    );
    assert_eq!(
        scope.run(id).map(|run| run.cleanup),
        Some(CleanupState::Complete(CleanupEvidence::BestEffortComplete))
    );
}
