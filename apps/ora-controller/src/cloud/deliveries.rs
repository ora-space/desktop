//! Revision delivery executions. Cloud stays the only durable authority for their registration,
//! results and receipts; the adapter adds only the checksums a Node reported, kept in process
//! memory so a reconnect can refresh checksum-bound grants. Upload grants themselves are relayed
//! and dropped: they are never stored, persisted or formatted into a log.
pub(super) mod mapping;
use super::{CloudStore, fault};
use crate::*;
use ora_controller_proto::v1 as proto;
use std::{collections::BTreeMap, sync::PoisonError};

impl CloudStore {
    /// Lists only the delivery family; clone, plugin and session polling keep their ownership.
    pub(super) async fn delivery_pending(
        &self,
        node: &NodeId,
    ) -> Result<Vec<DeliverRevisionMessage>, Error> {
        let response = fault::read(async {
            self.executions()
                .list_pending_dispatches(self.request(proto::ListPendingDispatchesRequest {
                    node_id: node.as_str().into(),
                }))
                .await
        })
        .await
        .map_err(|v| self.settle(v))?;
        response
            .records
            .iter()
            .filter(|r| mapping::is_delivery(r))
            .map(|r| mapping::deliver(r, node))
            .collect()
    }

    /// Proves operation, execution and target before deciding whether the record is a delivery.
    pub(super) async fn delivery_command(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<Option<DeliverRevisionMessage>, Error> {
        let record = self.owned_record(session, operation, execution).await?;
        if !mapping::is_delivery(&record) {
            return Ok(None);
        }
        Ok(Some(mapping::deliver(&record, &session.node_id)?))
    }

    /// The record of a delivery this Node owns, even after Cloud recorded its result.
    async fn delivery_record(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<(proto::ExecutionRecord, DeliverRevisionMessage), Error> {
        let record = self.owned_record(session, operation, execution).await?;
        if !mapping::is_delivery(&record) {
            return Err(Error::Conflict);
        }
        let command = mapping::deliver(&record, &session.node_id)?;
        Ok((record, command))
    }

    /// Reads a record whose Node and Node-local operation match the asking session.
    async fn owned_record(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<proto::ExecutionRecord, Error> {
        let record = self.record(execution).await?.ok_or(Error::Conflict)?;
        if record.node_id != session.node_id.as_str()
            || record.node_operation_id != operation.as_str()
        {
            return Err(Error::Conflict);
        }
        Ok(record)
    }

    /// Commits the exact terminal envelope and its receipt in Cloud before the session may ACK it.
    /// Cloud verifies the declared objects inside the same takeover, so a mismatch becomes its own
    /// verification verdict rather than anything this adapter decides.
    pub(super) async fn revision_event(
        &self,
        session: &NodeRuntimeIdentity,
        event: &RevisionResultMessage,
    ) -> Result<(), Error> {
        event.validate()?;
        if mapping::producer(&event.payload).node_id != session.node_id {
            return Err(Error::Conflict);
        }
        let (record, command) = self
            .delivery_record(session, &event.operation_id, &event.execution_id)
            .await?;
        if !mapping::accepts(&command, &event.payload) {
            return Err(Error::Conflict);
        }
        let epoch = self.epoch()?;
        let encoded = serde_json::to_vec(event)?;
        fault::write(|submission_id| {
            let request = proto::TakeOverNodeEventRequest {
                submission_id,
                epoch,
                operation_id: record.operation_id.clone(),
                execution_id: record.execution_id.clone(),
                sequence: event.sequence.value(),
                result: Some(mapping::result(&event.payload)),
                event: encoded.clone(),
            };
            async move {
                self.executions()
                    .take_over_node_event(self.request(request))
                    .await
            }
        })
        .await
        .map_err(|v| self.settle(v))?;
        self.forget_checksums(&event.execution_id);
        Ok(())
    }

    /// Requests checksum-bound grants for a running delivery. The adapter never asks for legacy
    /// grants without checksums: a grant fetched before the Node froze its objects could not bind
    /// their digests, so a resumed delivery without known digests waits for the Node to ask.
    pub(super) async fn upload_grants(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
        request: GrantRequest,
    ) -> Result<GrantOutcome, Error> {
        let checksums = match request {
            GrantRequest::Needed(checksums) => {
                self.checksum_memory()
                    .insert(execution.clone(), checksums.clone());
                checksums
            }
            GrantRequest::Resumed => match self.checksum_memory().get(execution).cloned() {
                Some(checksums) => checksums,
                None => return Ok(GrantOutcome::AwaitNode),
            },
        };
        let (record, command) = match self.delivery_record(session, operation, execution).await {
            Ok(found) => found,
            Err(Error::Conflict) => return Ok(self.not_grantable(execution)),
            Err(error) => return Err(error),
        };
        let spec = &command.payload.spec;
        if record.result.is_some()
            || checksums
                .keys()
                .any(|key| *key != spec.bundle_key && *key != spec.history_key)
        {
            return Ok(self.not_grantable(execution));
        }
        let epoch = self.epoch()?;
        let call = async {
            self.agents()
                .grant_revision_upload(self.request(proto::GrantRevisionUploadRequest {
                    epoch,
                    execution_id: record.execution_id.clone(),
                    checksums: mapping::checksums(&checksums),
                }))
                .await
        };
        // Cloud answers CONFLICT once the delivery has a result or its run left `delivering`; a
        // stale lease still drops the epoch through `settle` and is retried after re-acquiring.
        let response = match fault::read(call).await {
            Ok(response) => response,
            Err(fault::Verdict::Conflict | fault::Verdict::NotFound) => {
                return Ok(self.not_grantable(execution));
            }
            Err(verdict) => return Err(self.settle(verdict)),
        };
        Ok(GrantOutcome::Issued(mapping::grants(
            response, &command, &checksums,
        )?))
    }

    /// Records a grant refusal: the remembered digests can no longer be used for this execution.
    fn not_grantable(&self, execution: &ExecutionId) -> GrantOutcome {
        self.forget_checksums(execution);
        GrantOutcome::NotGrantable
    }

    fn forget_checksums(&self, execution: &ExecutionId) {
        self.checksum_memory().remove(execution);
    }

    /// The map holds no invariant a panic could break, so a poisoned lock is reused.
    fn checksum_memory(
        &self,
    ) -> std::sync::MutexGuard<
        '_,
        std::collections::HashMap<ExecutionId, BTreeMap<ObjectKey, Sha256Digest>>,
    > {
        self.inner
            .upload_checksums
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
