//! What the Controller does in each operation step, and how the sandbox targets follow the
//! snapshot. Every step either advances the operation or parks it; the next round claims again.
use super::{Carried, NODE_WAIT, POLL, Round, wait};
use crate::{
    cloud::{
        claim,
        fleet::{Binding, Gate, Sandbox},
        mapping, reports,
    },
    *,
};
use ora_controller_proto::v1::{self as proto, effect_evidence::Evidence};
use std::sync::Arc;
use tokio::time::Instant;

/// How long the Node may keep answering `Unknown` for the clone after its retransmission before
/// the outcome is treated as unknown: long enough for a Node that is still admitting the command.
const UNRESOLVED_GRACE: std::time::Duration = NODE_WAIT;

impl Round<'_> {
    /// The Workspace the operation is bound to; every step but a Project-wide one has one.
    fn workspace(&self) -> Result<&proto::OperationWorkspace, Error> {
        let id = self
            .operation
            .workspace_id
            .as_deref()
            .ok_or(Error::Conflict)?;
        self.snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.id == id)
            .ok_or(Error::Conflict)
    }

    /// The Workspaces an effect step acts on: the bound one, or every live one of the Project.
    fn scope(&self) -> Vec<proto::OperationWorkspace> {
        match &self.operation.workspace_id {
            Some(id) => self
                .snapshot
                .workspaces
                .iter()
                .filter(|workspace| &workspace.id == id)
                .cloned()
                .collect(),
            None => self.snapshot.workspaces.clone(),
        }
    }

    /// The Workspace's sandbox that has not been terminated, if any.
    fn live_sandbox(&self, workspace: &str) -> Option<proto::SandboxRecord> {
        self.snapshot
            .sandboxes
            .iter()
            .find(|sandbox| {
                sandbox.workspace_id == workspace && sandbox.observed_state != "terminated"
            })
            .cloned()
    }

    /// Starts targets for the Project's live sandboxes of the current generation and stops
    /// targets of terminated ones. A target needs the Node identity the ensure effect reported and
    /// the Substrate's sandbox identity; a sandbox whose ensure has not succeeded has neither yet.
    /// A sandbox being terminated is left to the terminate step, which stopped its session first.
    pub(super) async fn sync_fleet(&mut self) {
        for sandbox in &self.snapshot.sandboxes {
            match sandbox.observed_state.as_str() {
                "terminated" => self.fleet.remove(&sandbox.id).await,
                "terminating" => {}
                _ => {
                    let current = self.snapshot.workspaces.iter().any(|workspace| {
                        workspace.id == sandbox.workspace_id
                            && workspace.runtime_generation == sandbox.generation
                    });
                    let records: Vec<proto::NodeRecord> = self
                        .snapshot
                        .nodes
                        .iter()
                        .filter(|node| node.sandbox_instance_id == sandbox.id)
                        .cloned()
                        .collect();
                    if let (true, Some(binding)) = (current, self.binding(sandbox, &records)) {
                        self.fleet.ensure(binding, records);
                    }
                }
            }
        }
    }

    /// Derives a sandbox target from Cloud's record only: the ensure effect's evidence when this
    /// operation created the sandbox, otherwise a Node record Cloud accepted for it.
    fn binding(
        &self,
        sandbox: &proto::SandboxRecord,
        records: &[proto::NodeRecord],
    ) -> Option<Binding> {
        let ensure = self
            .snapshot
            .effects
            .iter()
            .find(|effect| effect.id == sandbox.id);
        let evidence = ensure.and_then(|effect| match &effect.evidence {
            Some(proto::EffectEvidence {
                evidence: Some(Evidence::SandboxEnsured(ensured)),
            }) => Some(ensured.node_id.clone()),
            _ => None,
        });
        let node_id = evidence.or_else(|| {
            records.iter().find_map(|record| {
                record
                    .identity
                    .as_ref()
                    .map(|identity| identity.node_id.clone())
            })
        })?;
        let external_id = sandbox
            .substrate_sandbox_id
            .clone()
            .or_else(|| ensure.and_then(|effect| effect.external_id.clone()))?;
        Some(Binding {
            sandbox_id: sandbox.id.clone(),
            generation: sandbox.generation,
            node_id: NodeId::new(node_id),
            external_id,
        })
    }

    /// sandbox: `sandbox_ensure` through the Substrate. The target is started by the next round,
    /// from the evidence Cloud recorded, so it never depends on anything only this round saw.
    pub(super) async fn sandbox(&mut self) -> Result<(), Error> {
        let workspace = self.workspace()?.id.clone();
        let effect = self
            .plan(proto::EffectKind::SandboxEnsure, &workspace)
            .await?;
        if let Carried::Parked = self.carry(effect).await? {
            return Ok(());
        }
        self.advance().await
    }

    /// The target of the Workspace's live sandbox once its Node is registered and connected, or
    /// `None` after parking the operation because the Node did not come up in time.
    pub(super) async fn ready_node(&mut self) -> Result<Option<Arc<Sandbox>>, Error> {
        let workspace = self.workspace()?.id.clone();
        let target = self
            .live_sandbox(&workspace)
            .and_then(|sandbox| self.fleet.get(&sandbox.id));
        if let Some(sandbox) = target
            && self.wait_ready(&sandbox).await
        {
            return Ok(Some(sandbox));
        }
        self.defer(
            proto::DeferState::RetryWait,
            proto::DeferReason::NodeUnavailable,
        )
        .await
        .map(|_| None)
    }

    /// Shares the first unavailable observation's deadline across every readiness check this round.
    async fn wait_ready(&mut self, sandbox: &Sandbox) -> bool {
        loop {
            if sandbox.report.lock().await.ready() {
                return true;
            }
            let deadline = self.node_grace.deadline();
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep_until((Instant::now() + POLL).min(deadline)).await;
        }
    }

    /// node: advances once Cloud has a registered, connected Node with a fresh heartbeat for the
    /// current generation, which the reporter provides after the handshake.
    pub(super) async fn node(&mut self) -> Result<(), Error> {
        if self.ready_node().await?.is_none() {
            return Ok(());
        }
        loop {
            match self.advance().await {
                // The heartbeat Cloud checks may lag the handshake by one report.
                Err(Error::Conflict) => {
                    let deadline = self.node_grace.deadline();
                    if Instant::now() < deadline {
                        tokio::time::sleep_until((Instant::now() + POLL).min(deadline)).await;
                        continue;
                    }
                    return self
                        .defer(
                            proto::DeferState::RetryWait,
                            proto::DeferReason::NodeUnavailable,
                        )
                        .await
                        .map(drop);
                }
                result => return result,
            }
        }
    }

    /// clone: dispatches one clone of the Binding snapshot Cloud froze for the step to the
    /// Workspace's Node and waits for its result. A pending execution from an earlier round is
    /// waited on, never registered twice; a failed one is followed by a new execution.
    pub(super) async fn clone_step(&mut self) -> Result<(), Error> {
        let latest = match self.snapshot.clones.last().cloned() {
            Some(record) => {
                let outcome = record.result.map(mapping::outcome).transpose()?;
                Some((ExecutionId::new(record.execution_id), outcome))
            }
            None => None,
        };
        let execution = match latest {
            Some((execution, None)) => execution,
            Some((_, Some(ExecutionOutcome::Ready { .. }))) => return self.node().await,
            Some((_, Some(ExecutionOutcome::Failed { .. }))) | None => {
                let Some(sandbox) = self.ready_node().await? else {
                    return Ok(());
                };
                match self.dispatch_clone(&sandbox).await? {
                    Some(execution) => execution,
                    None => return Ok(()),
                }
            }
        };
        let deadline = wait::clone_deadline();
        let workspace = self.workspace()?.id.clone();
        let sandbox = self
            .live_sandbox(&workspace)
            .and_then(|live| self.fleet.get(&live.id));
        loop {
            let record = self.store.record(&execution).await?;
            if let Some(result) = record.as_ref().and_then(|record| record.result.clone()) {
                return match mapping::outcome(result)? {
                    ExecutionOutcome::Ready { .. } => self.node().await,
                    ExecutionOutcome::Failed { failure, .. } => {
                        ora_logging::ora_warn!(execution_id = %execution.as_str(), failure = ?failure, "Workspace clone failed; a retry dispatches a new execution");
                        self.defer(
                            proto::DeferState::RetryWait,
                            proto::DeferReason::CloneFailed,
                        )
                        .await
                        .map(drop)
                    }
                };
            }
            let unresolved = sandbox
                .as_ref()
                .and_then(|sandbox| sandbox.unresolved_for(&execution));
            if unresolved.is_some_and(|elapsed| elapsed >= UNRESOLVED_GRACE) {
                ora_logging::ora_warn!(execution_id = %execution.as_str(), "the Node cannot tell the clone's outcome; the operation stays blocked");
                return self
                    .defer(
                        proto::DeferState::Blocked,
                        proto::DeferReason::CloneResultUnknown,
                    )
                    .await
                    .map(drop);
            }
            // Known results and existing Unknown classification win over the total deadline.
            if let Some(reason) = wait::clone_deadline_reason(
                record
                    .as_ref()
                    .and_then(|record| record.created_at.as_deref()),
                ora_logging::clock::now_local(),
                match unresolved {
                    Some(_) => wait::CloneEvidence::Unresolved,
                    None => wait::CloneEvidence::AwaitingResult,
                },
            ) {
                return self
                    .defer(proto::DeferState::Blocked, reason)
                    .await
                    .map(drop);
            }
            let Some(sandbox) = sandbox.as_ref() else {
                return self
                    .defer(
                        proto::DeferState::RetryWait,
                        proto::DeferReason::NodeUnavailable,
                    )
                    .await
                    .map(drop);
            };
            let unavailable_deadline =
                if !sandbox.connected() || !sandbox.report.lock().await.ready() {
                    Some(self.node_grace.deadline())
                } else {
                    None
                };
            if unavailable_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return self
                    .defer(
                        proto::DeferState::RetryWait,
                        proto::DeferReason::NodeUnavailable,
                    )
                    .await
                    .map(drop);
            }
            if Instant::now() >= deadline {
                return self
                    .defer(
                        proto::DeferState::RetryWait,
                        proto::DeferReason::CloneResultPending,
                    )
                    .await
                    .map(drop);
            }
            let wake = (Instant::now() + POLL).min(deadline);
            tokio::time::sleep_until(
                unavailable_deadline.map_or(wake, |unavailable| wake.min(unavailable)),
            )
            .await;
        }
    }

    /// Registers the Workspace's clone with Cloud while dispatch to the Node is open, forwarding
    /// the snapshot input Cloud froze (`clone_input`) verbatim; the Controller never builds a
    /// clone input from the Project or Workspace fields. A step without that input, or with one
    /// the Node protocol refuses, blocks the operation instead of being sent: retrying cannot make
    /// it valid, and both mean Cloud handed out a step it should not have.
    async fn dispatch_clone(&mut self, sandbox: &Sandbox) -> Result<Option<ExecutionId>, Error> {
        let refusal = match self.snapshot.clone_input.as_ref() {
            None => "the clone step carries no frozen clone input",
            Some(input) => match mapping::clone_spec(input, &sandbox.binding.node_id) {
                Err(_) => "the frozen clone input is not a valid Node command",
                Ok(spec) => {
                    let operation = OperationId::new(self.operation.id.clone());
                    let gate = sandbox.gate.lock().await;
                    if *gate == Gate::Closed {
                        return Err(Error::Conflict);
                    }
                    match claim::record_dispatch(self.store, self.epoch, operation, spec).await {
                        Ok(command) => return Ok(Some(command.execution_id)),
                        Err(Error::Validation(_)) => "the Node protocol refuses the clone command",
                        Err(error) => return Err(error),
                    }
                }
            },
        };
        ora_logging::ora_error!(
            operation_id = %self.operation.id,
            reason = %refusal,
            "the clone step is not dispatched; the operation stays blocked"
        );
        self.defer(
            proto::DeferState::Blocked,
            proto::DeferReason::ExternalFailure,
        )
        .await
        .map(|_| None)
    }

    /// quiesce: for each Workspace in scope, stops dispatching to its Node, then reports idle only
    /// when no dispatch to it lacks a terminal result. Cloud refusing idle fails the operation and
    /// restores admission, so dispatch reopens.
    pub(super) async fn quiesce(&mut self) -> Result<(), Error> {
        for workspace in self.scope() {
            let Some(live) = self.live_sandbox(&workspace.id) else {
                continue;
            };
            let Some(sandbox) = self.fleet.get(&live.id) else {
                self.defer(
                    proto::DeferState::RetryWait,
                    proto::DeferReason::NodeUnavailable,
                )
                .await?;
                return Ok(());
            };
            if !self.wait_ready(&sandbox).await {
                self.defer(
                    proto::DeferState::RetryWait,
                    proto::DeferReason::NodeUnavailable,
                )
                .await?;
                return Ok(());
            }
            *sandbox.gate.lock().await = Gate::Closed;
            let pending = self
                .store
                .pending_dispatches(&sandbox.binding.node_id)
                .await?;
            let plugin_pending = self.store.pending_plugins(&sandbox.binding.node_id).await?;
            let agent_pending = self.store.pending_agents(&sandbox.binding.node_id).await?;
            let delivery_pending = self
                .store
                .pending_deliveries(&sandbox.binding.node_id)
                .await?;
            let idle = pending.is_empty()
                && plugin_pending.is_empty()
                && agent_pending.is_empty()
                && delivery_pending.is_empty()
                && !sandbox.any_unresolved();
            let accepted = reports::idle(
                self.store,
                &sandbox,
                &self.operation.id,
                workspace.admission_epoch,
                idle,
            )
            .await?;
            if !accepted {
                *sandbox.gate.lock().await = Gate::Open;
                ora_logging::ora_info!(operation_id = %self.operation.id, "Node was not idle; Cloud failed the operation and reopened admission");
                return Ok(());
            }
        }
        self.advance().await
    }

    /// terminate: for each Workspace in scope with a live sandbox, stops its session and removes
    /// its target before `sandbox_terminate` is even planned, so the Controller never reconnects
    /// to a sandbox being destroyed.
    pub(super) async fn terminate(&mut self) -> Result<(), Error> {
        for workspace in self.scope() {
            let Some(live) = self.live_sandbox(&workspace.id) else {
                continue;
            };
            self.fleet.remove(&live.id).await;
            let effect = self
                .plan(proto::EffectKind::SandboxTerminate, &workspace.id)
                .await?;
            if let Carried::Parked = self.carry(effect).await? {
                return Ok(());
            }
        }
        self.advance().await
    }

    /// cleanup: deletes each Workspace's data once its sandbox is gone.
    pub(super) async fn cleanup(&mut self) -> Result<(), Error> {
        for workspace in self.scope() {
            let effect = self
                .plan(proto::EffectKind::WorkspaceDataDelete, &workspace.id)
                .await?;
            if let Carried::Parked = self.carry(effect).await? {
                return Ok(());
            }
        }
        self.advance().await
    }
}
