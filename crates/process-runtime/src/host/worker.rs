use std::path::Path;

use ora_process_client::{GuardianManagement, GuardianRuns};
use ora_process_protocol::{
    CleanupEvidence, CleanupState, DirectProcessState, GuardianAccess, GuardianHostSession,
    GuardianManagementReply, GuardianRunOperation, GuardianRunRejection, GuardianRunResult,
    HostBinding, HostCoordination, HostRunIntent, LaunchFact, RunSnapshot, ScopeId, ScopeState,
};

use crate::host_state::launch::GuardianLaunch;
use crate::{HostState, ProcessStateError};

pub(super) struct Observation {
    pub(super) scope: Option<ScopeState>,
    pub(super) runs: Vec<RunSnapshot>,
    pub(super) status: HostCoordination,
}

pub(super) struct Work {
    access: Option<GuardianAccess>,
    launch: Option<GuardianLaunch>,
    binding: HostBinding,
    owner: u32,
    close: bool,
    runs: Vec<(HostRunIntent, bool)>,
}

/// Snapshots accepted responsibilities; new control intent is handled by the next independent pass.
pub(super) fn prepare(
    state: &mut HostState,
    scope: ScopeId,
    guardian: &Path,
    owner: u32,
) -> Result<Work, ProcessStateError> {
    let close = state.scope_close_requested(scope)?;
    let mut runs = Vec::new();
    for intent in state
        .run_intents()?
        .into_iter()
        .filter(|intent| intent.scope == scope)
    {
        let stop = state.run_stop_requested(intent.run)?;
        runs.push((intent, stop));
    }
    let mut launch = None;
    if !close && state.guardian_access(scope)?.is_none() {
        // Failure after the journal commit consumes launch permission. Discovery below retains it.
        // Preflight failures remain retriable; neither path deletes files or replaces identities.
        launch = state.begin_guardian(scope, guardian).ok();
    }
    Ok(Work {
        access: state.guardian_access(scope)?,
        launch,
        binding: state.binding(),
        owner,
        close,
        runs,
    })
}

impl Work {
    /// A transport failure preserves collected evidence but never means an unacknowledged Start failed.
    pub(super) async fn observe(mut self) -> Observation {
        let mut observation = Observation {
            scope: None,
            runs: Vec::new(),
            status: HostCoordination::Observing,
        };
        if let Some(launch) = self.launch.take() {
            let _ = launch.deliver().await;
        }
        let Some(access) = self.access else {
            if self.close {
                observation.scope = Some(ScopeState::Closed(CleanupEvidence::BestEffortComplete));
                observation.runs = self
                    .runs
                    .iter()
                    .map(|(intent, _)| cancelled(intent.run))
                    .collect();
            } else {
                observation.status = HostCoordination::Unavailable;
            }
            return observation;
        };
        let management = GuardianManagement::new(access.clone(), self.owner);
        let session = match management.bind(self.binding).await {
            Ok(GuardianManagementReply::Bound { session, .. }) => session,
            Ok(GuardianManagementReply::Rejected(reason)) => {
                observation.status =
                    HostCoordination::Rejected(GuardianRunRejection::Management(reason));
                return observation;
            }
            Ok(GuardianManagementReply::Current { .. }) | Err(_) => {
                observation.status = HostCoordination::Unavailable;
                return observation;
            }
        };
        let client = GuardianRuns::new(
            access,
            self.owner,
            GuardianHostSession { host: session.host },
        );
        let scope_operation = if self.close {
            GuardianRunOperation::Close
        } else {
            GuardianRunOperation::Scope
        };
        match client.execute(scope_operation).await {
            Ok(GuardianRunResult::Scope(scope)) => observation.scope = Some(scope),
            Ok(GuardianRunResult::Rejected(reason)) => {
                observation.status = HostCoordination::Rejected(reason);
                return observation;
            }
            Ok(GuardianRunResult::Run(_) | GuardianRunResult::Output { .. }) | Err(_) => {
                observation.status = HostCoordination::Unavailable;
                return observation;
            }
        }
        for (intent, stop) in self.runs {
            let operation = if stop {
                GuardianRunOperation::Stop { run: intent.run }
            } else {
                intent.start_operation()
            };
            match client.execute(operation).await {
                Ok(GuardianRunResult::Run(snapshot)) => observation.runs.push(snapshot),
                Ok(GuardianRunResult::Rejected(GuardianRunRejection::UnknownRun)) if stop => {
                    // Current host binding fences old requests; this worker never queues Start after Stop.
                    observation.runs.push(cancelled(intent.run));
                }
                Ok(GuardianRunResult::Rejected(reason)) => {
                    observation.status = HostCoordination::Rejected(reason)
                }
                Ok(GuardianRunResult::Scope(_) | GuardianRunResult::Output { .. }) | Err(_) => {
                    observation.status = HostCoordination::Unavailable;
                    break;
                }
            }
        }
        observation
    }
}

/// Only a never-attempted guardian or its authoritative UnknownRun after fencing permits this fact.
fn cancelled(run: ora_process_protocol::RunId) -> RunSnapshot {
    RunSnapshot {
        id: run,
        launch: LaunchFact::NotStarted("cancelled before guardian acceptance".into()),
        direct: DirectProcessState::NotStarted,
        cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete),
    }
}
