//! The memory-only download grant exchange of a session that restores its prior Revision.
//!
//! Like upload grants, download grants never enter the execution input: a grant expires while a
//! session input stays fixed. Neither message carries a sequence, is persisted, or is logged. The
//! Node repeats its request on every new Controller connection, so the Controller keeps nothing.

use super::validation::{validate_execution_ids, validate_identity, validate_protocol_version};
use crate::{
    ExecutionId, MessageValidationError, NodeId, ObjectDownloadGrant, OperationId, ProtocolVersion,
    ValidateMessage,
};
use serde::{Deserialize, Serialize};

/// The session holds no valid grant for its prior bundle (never received, expired, or refused by
/// the object store); the key is the one fixed in the session input, so the request names none.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadGrantNeeded {
    pub node_id: NodeId,
}

/// The Controller's answer to a download grant request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum DownloadGrant {
    /// Fresh grants, replacing any the Node held.
    Granted { grants: Vec<ObjectDownloadGrant> },
    /// Cloud will not grant the read: the session cannot restore and ends as failed.
    Refused {},
}

/// Asks the Controller for a fresh grant; the session keeps waiting within its restore deadline.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DownloadGrantNeededMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: DownloadGrantNeeded,
}

/// Hands a restoring session its download grant, or tells it none will come.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DownloadGrantMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: DownloadGrant,
}

impl ValidateMessage for DownloadGrantNeededMessage {
    /// Requires the requesting Node, so the Controller can refuse a request for another Node.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        validate_identity(self.payload.node_id.is_empty(), "node_id")
    }
}

impl ValidateMessage for DownloadGrantMessage {
    /// A granted answer carries at least one usable grant; matching its key against the session
    /// input is the Node's job.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        match &self.payload {
            DownloadGrant::Granted { grants } => {
                if grants.is_empty() {
                    return Err(MessageValidationError::EmptyDownloadGrant);
                }
                grants.iter().try_for_each(ObjectDownloadGrant::validate)
            }
            DownloadGrant::Refused {} => Ok(()),
        }
    }
}
