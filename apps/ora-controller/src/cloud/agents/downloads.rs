//! Read grants for the prior bundle of a session that resumes a Revision (restore contract D2,
//! D5). Nothing is remembered between requests: the Node asks again on every new connection, and
//! issued grants pass straight to the session that asked, never into a log or the store.
use super::super::{CloudStore, fault};
use super::mapping;
use crate::*;
use ora_controller_proto::v1 as proto;

impl CloudStore {
    /// Grants the read when Cloud still considers the session restoring, and refuses when Cloud
    /// definitively declines. An outage or a lost lease is an error the relay retries: a refusal
    /// fails the Node's restore and therefore its run, which an outage must never cause.
    pub(in crate::cloud) async fn download_grants(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<DownloadGrantMessage, Error> {
        let refused = || DownloadGrantMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: operation.clone(),
            execution_id: execution.clone(),
            payload: DownloadGrant::Refused {},
        };
        // A session this Node does not own, or that is not a session, is never granted.
        let record = match self.agent_record(session, operation, execution).await {
            Ok(record) => record,
            Err(Error::Conflict) => return Ok(refused()),
            Err(error) => return Err(error),
        };
        let command = mapping::start(&record, &session.node_id)?;
        if record.result.is_some() || command.payload.spec.prior_revision.is_none() {
            return Ok(refused());
        }
        let epoch = self.epoch()?;
        let call = async {
            self.agents()
                .grant_revision_download(self.request(proto::GrantRevisionDownloadRequest {
                    epoch,
                    execution_id: record.execution_id.clone(),
                }))
                .await
        };
        // Cloud answers NOT_FOUND for an unknown execution and ABORTED (CONFLICT) once the session
        // has a result, its run stopped, or the input names no bundle. FAILED_PRECONDITION is a
        // stale lease: `settle` drops the epoch and the request is retried after re-acquiring.
        match fault::read(call).await {
            Ok(response) => mapping::download_grants(response, &command),
            Err(fault::Verdict::Conflict | fault::Verdict::NotFound) => Ok(refused()),
            Err(verdict) => Err(self.settle(verdict)),
        }
    }
}
