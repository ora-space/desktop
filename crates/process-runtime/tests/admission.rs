use ora_process_protocol::{ContainmentGuarantee, ContainmentRequest, RunId, RunSpec};
use ora_process_runtime::{
    AdmissionError, Platform, PlatformCapabilities, ScopeRuntime, SpawnError,
};
use ora_process_runtime::{PlatformError, PlatformObservation, StopSignal};
use pretty_assertions::assert_eq;

struct AvailablePlatform(PlatformCapabilities);

impl Platform for AvailablePlatform {
    /// Supplies deployment facts without deriving capability from an OS name.
    fn capabilities(&self) -> PlatformCapabilities {
        self.0
    }

    /// Prevents admission-only tests from accidentally launching workloads.
    fn spawn(
        &mut self,
        _run: RunId,
        _spec: &RunSpec,
        _guarantee: ContainmentGuarantee,
    ) -> Result<(), SpawnError> {
        Err(SpawnError::NotStarted("no launcher configured".into()))
    }

    /// No workload exists in this admission fixture.
    fn observe(&mut self, _run: RunId) -> Result<PlatformObservation, PlatformError> {
        Err(PlatformError("no workload".into()))
    }

    /// No workload exists in this admission fixture.
    fn signal(&mut self, _run: RunId, _signal: StopSignal) -> Result<(), PlatformError> {
        Err(PlatformError("no workload".into()))
    }
}

/// specs/test-cases/node/process/containment/guarantees/capability-and-evidence.md#containment-guarantees-are-fixed-before-business-execution
#[test]
fn containment_selection_respects_required_and_explicit_guarantees() {
    use ContainmentGuarantee::{BestEffort, Strong};
    use ContainmentRequest::{BestEffort as RequestBestEffort, PreferStrong, RequireStrong};
    use PlatformCapabilities::{BestEffortOnly, StrongAndBestEffort, StrongOnly, Unavailable};

    let scenarios = [
        (
            RequireStrong,
            BestEffortOnly,
            Err(AdmissionError::Unavailable),
        ),
        (PreferStrong, BestEffortOnly, Ok(BestEffort)),
        (RequireStrong, StrongOnly, Ok(Strong)),
        (PreferStrong, StrongAndBestEffort, Ok(Strong)),
        (RequestBestEffort, StrongAndBestEffort, Ok(BestEffort)),
        (
            RequestBestEffort,
            StrongOnly,
            Err(AdmissionError::Unavailable),
        ),
        (PreferStrong, Unavailable, Err(AdmissionError::Unavailable)),
    ];
    for (request, capabilities, expected) in scenarios {
        let actual = ScopeRuntime::new(request, AvailablePlatform(capabilities))
            .map(|scope| scope.guarantee());
        assert_eq!(actual, expected);
    }
}
