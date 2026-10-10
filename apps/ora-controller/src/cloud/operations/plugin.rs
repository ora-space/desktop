//! The plugin step dispatches one Cloud-owned input and advances only from durable evidence.
use super::{NODE_WAIT, POLL, Round};
use crate::{cloud::fleet::Gate, *};
use ora_controller_proto::v1 as proto;
use std::time::Instant;

impl Round<'_> {
    /// Individual plugin failures are completed evidence; only whole-execution failures retry.
    pub(super) async fn plugin(&mut self) -> Result<(), Error> {
        let Some(input) = self.snapshot.plugin_input.clone() else {
            return self.advance().await;
        };
        let latest = self.snapshot.plugin_executions.last().cloned();
        if let Some(record) = &latest {
            if record.operation_id != self.operation.id || record.input.as_ref() != Some(&input) {
                return Err(Error::Conflict);
            }
            if matches!(
                record.result.as_ref().and_then(|r| r.outcome.as_ref()),
                Some(proto::execution_result::Outcome::PluginsResult(_))
            ) {
                return self.advance().await;
            }
        }
        let Some(sandbox) = self.ready_node().await? else {
            return Ok(());
        };
        if !sandbox.advertises(NodeCapability::PluginInstall) {
            return self
                .defer(
                    proto::DeferState::Blocked,
                    proto::DeferReason::ExternalFailure,
                )
                .await
                .map(drop);
        }
        let execution = match latest {
            Some(record) if record.result.is_none() => {
                if record.node_id != sandbox.binding.node_id.as_str() {
                    return Err(Error::Conflict);
                }
                ExecutionId::new(record.execution_id)
            }
            Some(record)
                if !matches!(
                    record.result.as_ref().and_then(|r| r.outcome.as_ref()),
                    Some(proto::execution_result::Outcome::PluginsFailed(_))
                ) =>
            {
                return Err(Error::Conflict);
            }
            _ => {
                let registration = {
                    let gate = sandbox.gate.lock().await;
                    if *gate == Gate::Closed {
                        return Err(Error::Conflict);
                    }
                    self.store
                        .record_plugins(
                            self.epoch,
                            &self.operation.id,
                            &sandbox.binding.node_id,
                            &input,
                        )
                        .await
                };
                match registration {
                    Ok(command) => command.execution_id().clone(),
                    Err(Error::Validation(_) | Error::Configuration(_)) => {
                        return self
                            .defer(
                                proto::DeferState::Blocked,
                                proto::DeferReason::ExternalFailure,
                            )
                            .await
                            .map(drop);
                    }
                    Err(error) => return Err(error),
                }
            }
        };
        let mut disconnected: Option<Instant> = None;
        loop {
            if let Some(result) = self.store.record(&execution).await?.and_then(|r| r.result) {
                return match result.outcome {
                    Some(proto::execution_result::Outcome::PluginsResult(_)) => {
                        self.advance().await
                    }
                    Some(proto::execution_result::Outcome::PluginsFailed(_)) => self
                        .defer(
                            proto::DeferState::RetryWait,
                            proto::DeferReason::PluginExecutionFailed,
                        )
                        .await
                        .map(drop),
                    _ => Err(Error::Conflict),
                };
            }
            if sandbox
                .unresolved_for(&execution)
                .is_some_and(|elapsed| elapsed >= NODE_WAIT)
            {
                return self
                    .defer(
                        proto::DeferState::Blocked,
                        proto::DeferReason::PluginResultUnknown,
                    )
                    .await
                    .map(drop);
            }
            if sandbox.connected() {
                disconnected = None;
            } else if disconnected.get_or_insert_with(Instant::now).elapsed() >= NODE_WAIT {
                return self
                    .defer(
                        proto::DeferState::RetryWait,
                        proto::DeferReason::PluginExecutionFailed,
                    )
                    .await
                    .map(drop);
            }
            tokio::time::sleep(POLL).await;
        }
    }
}
