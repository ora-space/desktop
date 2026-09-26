//! Drives Cloud's runtime Workspace operations: create goes sandbox, node, clone; start goes
//! sandbox, node; stop and delete go quiesce, terminate (and cleanup). Cloud decides every step,
//! effect and input; the Controller only carries each step out and reports what it observed.
//!
//! `ClaimOperation` always returns the oldest claimable operation, and calling it again returns a
//! running one with a new version, so one Controller drives one operation at a time. Each round
//! claims, takes the current step as far as it can and either advances or parks the operation;
//! the next round claims again and starts from Cloud's fresh snapshot. The task that runs the
//! rounds belongs to the lease it was started under and is stopped when that lease is lost.
mod steps;

use super::{CloudStore, fault, fleet::Fleet, substrate::Observation};
use crate::*;
use ora_controller_proto::v1::{
    self as proto, workspace_operation_service_client::WorkspaceOperationServiceClient,
};
use std::time::Duration;
use tokio::task::JoinHandle;
use tonic::transport::Channel;

/// How long a parked operation waits before Cloud offers it again.
const RETRY_SECONDS: u32 = 5;

/// The operation task of the current lease, if one runs.
#[derive(Default)]
pub(super) struct Operations {
    task: Option<(i64, JoinHandle<()>)>,
}

impl Operations {
    /// Starts claiming under the held lease unless a task for it is still running. Every claim
    /// trigger ends here, so signals, renewals and the fallback tick all reach operations.
    pub(super) fn wake(&mut self, store: &CloudStore, fleet: &Fleet) {
        let Some(epoch) = store.lease() else {
            return;
        };
        if let Some((running, task)) = &self.task
            && *running == epoch
            && !task.is_finished()
        {
            return;
        }
        self.stop();
        let (store, fleet) = (store.clone(), fleet.clone());
        self.task = Some((
            epoch,
            tokio::spawn(async move { drain(&store, &fleet, epoch).await }),
        ));
    }

    /// Stops the task at once: without the lease nothing it would write can be accepted, and a
    /// successor resumes the operation from Cloud's record. Node sessions are not affected.
    pub(super) fn stop(&mut self) {
        if let Some((_, task)) = self.task.take() {
            task.abort();
        }
    }
}

/// Claims and drives operations until Cloud has none to offer, a round fails, or the lease moves.
async fn drain(store: &CloudStore, fleet: &Fleet, epoch: i64) {
    while store.lease() == Some(epoch) {
        let snapshot = match claim(store, epoch).await {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return,
            Err(error) => {
                ora_logging::ora_warn!(error = %error, "Cloud operation claim failed");
                return;
            }
        };
        let Some(mut round) = Round::new(store, fleet, epoch, snapshot) else {
            ora_logging::ora_warn!("Cloud returned an operation snapshot without its operation");
            return;
        };
        let operation = round.operation.id.clone();
        // A failed round leaves the operation running; the next trigger claims it again and
        // starts over from Cloud's record, so nothing is retried here.
        if let Err(error) = round.run().await {
            ora_logging::ora_warn!(operation_id = %operation, error = %error, "operation round failed; it resumes on the next claim");
            return;
        }
    }
}

impl CloudStore {
    fn operations(&self) -> WorkspaceOperationServiceClient<Channel> {
        WorkspaceOperationServiceClient::new(self.inner.channel.clone())
    }
}

/// `ClaimOperation`. It carries no submission identity: a repeated claim returns the same running
/// operation with a newer version, and the caller always works from the latest snapshot.
async fn claim(store: &CloudStore, epoch: i64) -> Result<Option<proto::OperationSnapshot>, Error> {
    let call = async {
        let request = store.request(proto::ClaimOperationRequest { epoch });
        store.operations().claim_operation(request).await
    };
    // A lost claim reply commits nothing a later claim would not repeat, so it is a read.
    fault::read(call)
        .await
        .map(|response| response.snapshot)
        .map_err(|verdict| store.settle(verdict))
}

/// Whether the round may go on after carrying effects to the Substrate.
enum Carried {
    /// Every effect involved succeeded (or was reconciled) and Cloud recorded it.
    Done,
    /// The operation was parked in `retry_wait` or `blocked`; the round ends.
    Parked,
}

/// One claimed operation and the snapshot it was claimed with. Writes update `operation`, so each
/// one presents the version the previous one returned.
struct Round<'a> {
    store: &'a CloudStore,
    fleet: &'a Fleet,
    epoch: i64,
    operation: proto::Operation,
    snapshot: proto::OperationSnapshot,
}

impl<'a> Round<'a> {
    fn new(
        store: &'a CloudStore,
        fleet: &'a Fleet,
        epoch: i64,
        mut snapshot: proto::OperationSnapshot,
    ) -> Option<Self> {
        let operation = snapshot.operation.take()?;
        Some(Self {
            store,
            fleet,
            epoch,
            operation,
            snapshot,
        })
    }

    /// Reconciles effects of earlier epochs, brings the sandbox targets in line with the snapshot,
    /// then runs the current step.
    async fn run(&mut self) -> Result<(), Error> {
        if let Carried::Parked = self.reconcile().await? {
            return Ok(());
        }
        match self.operation.step() {
            // Tearing down: connecting to a sandbox about to be terminated would only be undone.
            proto::OperationStep::Terminate | proto::OperationStep::Cleanup => {}
            _ => self.sync_fleet().await,
        }
        match self.operation.step() {
            proto::OperationStep::Sandbox => self.sandbox().await,
            proto::OperationStep::Node => self.node().await,
            proto::OperationStep::Clone => self.clone_step().await,
            proto::OperationStep::Quiesce => self.quiesce().await,
            proto::OperationStep::Terminate => self.terminate().await,
            proto::OperationStep::Cleanup => self.cleanup().await,
            proto::OperationStep::Plugin => self.plugin().await,
            proto::OperationStep::Done | proto::OperationStep::Unspecified => Err(Error::Conflict),
        }
    }

    /// Cloud advances only when every effect of the operation was reconciled under the current
    /// epoch, so effects a previous holder planned are queried by their ID and reported first.
    async fn reconcile(&mut self) -> Result<Carried, Error> {
        let stale: Vec<proto::Effect> = self
            .snapshot
            .effects
            .iter()
            .filter(|effect| effect.reconciled_epoch != self.epoch)
            .cloned()
            .collect();
        for effect in stale {
            match self.fleet.deployment().substrate.get(&effect).await {
                Ok(Some(observation)) => self.record(&effect.id, Some(&observation)).await?,
                Ok(None) => self.record(&effect.id, None).await?,
                Err(error) => return self.park_on(&effect, &error).await,
            }
        }
        Ok(Carried::Done)
    }

    /// `PlanEffect` for the current step; planning again returns the original effect.
    async fn plan(
        &mut self,
        kind: proto::EffectKind,
        workspace: &str,
    ) -> Result<proto::Effect, Error> {
        let (store, epoch) = (self.store, self.epoch);
        let (operation, version) = (self.operation.id.clone(), self.operation.version);
        let response = fault::write(|submission_id| {
            let operation = operation.clone();
            async move {
                let request = store.request(proto::PlanEffectRequest {
                    submission_id,
                    epoch,
                    operation_id: operation,
                    version,
                    kind: kind as i32,
                    workspace_id: workspace.into(),
                });
                store.operations().plan_effect(request).await
            }
        })
        .await
        .map_err(|verdict| store.settle(verdict))?;
        self.update(response.operation)?;
        response.effect.ok_or(Error::Conflict)
    }

    /// `RecordEffectResult`; `None` reports that the Substrate has no record of the effect.
    async fn record(
        &mut self,
        effect: &str,
        observation: Option<&Observation>,
    ) -> Result<(), Error> {
        let (state, external_id, evidence, failure) = match observation {
            None => (proto::EffectState::Absent, String::new(), None, None),
            Some(Observation::Running { external_id }) => {
                (proto::EffectState::Running, external_id.clone(), None, None)
            }
            Some(Observation::Succeeded {
                external_id,
                evidence,
            }) => (
                proto::EffectState::Succeeded,
                external_id.clone(),
                Some(evidence.clone()),
                None,
            ),
            Some(Observation::Failed {
                external_id,
                failure,
            }) => (
                proto::EffectState::Failed,
                external_id.clone(),
                None,
                failure.clone(),
            ),
        };
        let (store, epoch) = (self.store, self.epoch);
        let (operation, version) = (self.operation.id.clone(), self.operation.version);
        let response = fault::write(|submission_id| {
            let request = proto::RecordEffectResultRequest {
                submission_id,
                epoch,
                operation_id: operation.clone(),
                version,
                effect_id: effect.into(),
                state: state as i32,
                external_id: external_id.clone(),
                evidence: evidence.clone(),
                failure: failure.clone(),
            };
            async move {
                let request = store.request(request);
                store.operations().record_effect_result(request).await
            }
        })
        .await
        .map_err(|verdict| store.settle(verdict))?;
        self.update(response.operation)
    }

    /// `AdvanceOperation`.
    async fn advance(&mut self) -> Result<(), Error> {
        let (store, epoch) = (self.store, self.epoch);
        let (operation, version) = (self.operation.id.clone(), self.operation.version);
        let response = fault::write(|submission_id| {
            let operation = operation.clone();
            async move {
                let request = store.request(proto::AdvanceOperationRequest {
                    submission_id,
                    epoch,
                    operation_id: operation,
                    version,
                });
                store.operations().advance_operation(request).await
            }
        })
        .await
        .map_err(|verdict| store.settle(verdict))?;
        self.update(response.operation)?;
        ora_logging::ora_info!(
            operation_id = %self.operation.id,
            step = self.operation.step().as_str_name(),
            "operation advanced"
        );
        Ok(())
    }

    /// `DeferOperation`: parks the operation with a finite reason and ends the round.
    async fn defer(
        &mut self,
        state: proto::DeferState,
        reason: proto::DeferReason,
    ) -> Result<Carried, Error> {
        let (store, epoch) = (self.store, self.epoch);
        let (operation, version) = (self.operation.id.clone(), self.operation.version);
        let response = fault::write(|submission_id| {
            let operation = operation.clone();
            async move {
                let request = store.request(proto::DeferOperationRequest {
                    submission_id,
                    epoch,
                    operation_id: operation,
                    version,
                    state: state as i32,
                    reason: reason as i32,
                    retry_seconds: RETRY_SECONDS,
                });
                store.operations().defer_operation(request).await
            }
        })
        .await
        .map_err(|verdict| store.settle(verdict))?;
        self.update(response.operation)?;
        ora_logging::ora_info!(
            operation_id = %self.operation.id,
            state = state.as_str_name(),
            reason = reason.as_str_name(),
            "operation parked"
        );
        Ok(Carried::Parked)
    }

    fn update(&mut self, operation: Option<proto::Operation>) -> Result<(), Error> {
        self.operation = operation.ok_or(Error::Conflict)?;
        Ok(())
    }

    /// Carries one planned effect to the Substrate and records what it reported. An effect already
    /// succeeded and reconciled under this epoch is not sent again.
    async fn carry(&mut self, effect: proto::Effect) -> Result<Carried, Error> {
        if effect.state() == proto::EffectState::Succeeded && effect.reconciled_epoch == self.epoch
        {
            return Ok(Carried::Done);
        }
        let observation = match self.fleet.deployment().substrate.execute(&effect).await {
            Ok(observation) => observation,
            Err(error) => return self.park_on(&effect, &error).await,
        };
        self.record(&effect.id, Some(&observation)).await?;
        match observation {
            Observation::Succeeded { .. } => Ok(Carried::Done),
            Observation::Failed { failure, .. } => {
                ora_logging::ora_warn!(effect_id = %effect.id, failure = failure.as_deref().unwrap_or("unspecified"), "Substrate reported a failed effect");
                self.defer(
                    proto::DeferState::RetryWait,
                    proto::DeferReason::ExternalFailure,
                )
                .await
            }
            // A terminate still in progress blocks: a new generation may not start before the old
            // one is confirmed gone.
            Observation::Running { .. } if effect.kind() == proto::EffectKind::SandboxTerminate => {
                self.defer(
                    proto::DeferState::Blocked,
                    proto::DeferReason::TerminationUnconfirmed,
                )
                .await
            }
            Observation::Running { .. } => {
                self.defer(
                    proto::DeferState::RetryWait,
                    proto::DeferReason::SubstrateTimeout,
                )
                .await
            }
        }
    }

    /// Parks the operation for an effect that produced no observation.
    async fn park_on(
        &mut self,
        effect: &proto::Effect,
        error: &super::substrate::SubstrateError,
    ) -> Result<Carried, Error> {
        use super::substrate::SubstrateError;
        ora_logging::ora_warn!(effect_id = %effect.id, error = %error, "Substrate effect call failed");
        match error {
            SubstrateError::Unreachable(_) => {
                self.defer(
                    proto::DeferState::RetryWait,
                    proto::DeferReason::SubstrateTimeout,
                )
                .await
            }
            SubstrateError::Rejected(_) => {
                self.defer(
                    proto::DeferState::Blocked,
                    proto::DeferReason::ExternalFailure,
                )
                .await
            }
        }
    }
}

/// How long the node, clone and quiesce steps wait for a Node before parking the operation.
const NODE_WAIT: Duration = Duration::from_secs(/*secs*/ 60);
/// How often a waiting step looks again.
const POLL: Duration = Duration::from_millis(/*millis*/ 250);
