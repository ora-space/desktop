//! Revision delivery: its command, terminal event, and the memory-only upload grant exchange.
//!
//! Upload grants never enter the execution input: a grant expires, while the same execution must
//! be resendable with identical input. Neither `UploadGrant` nor `UploadGrantNeeded` carries a
//! sequence, is persisted, or is logged.

use super::validation::{validate_execution_ids, validate_identity, validate_protocol_version};
use crate::{
    DeliverRevisionSpec, ExecutionId, MessageValidationError, NodeId, ObjectKey, ObjectUploadGrant,
    OperationId, ProtocolVersion, RevisionExecutionResult, Sequence, Sha256Digest, ValidateMessage,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Saves an ended session's Workspace and history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeliverRevision {
    pub spec: DeliverRevisionSpec,
}

/// Fresh grants for a running delivery, replacing any the Node held.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UploadGrant {
    pub grants: Vec<ObjectUploadGrant>,
}

/// The Node must upload but holds no valid grant (never received, expired, or lost on restart).
///
/// `checksums` names every object the Node still has to upload with the SHA-256 it froze before
/// the first PUT, so the Controller can ask Cloud for checksum-bound grants
/// (`GrantRevisionUploadRequest.checksums`): the store then refuses any other bytes under the key.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UploadGrantNeeded {
    pub node_id: NodeId,
    pub checksums: BTreeMap<ObjectKey, Sha256Digest>,
}

/// Correlates a delivery with its IssueRun and durable execution identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeliverRevisionMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: DeliverRevision,
}

/// One retained terminal delivery event; status queries reuse its payload without acknowledging it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RevisionResultMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub sequence: Sequence,
    pub payload: RevisionExecutionResult,
}

/// Hands a running delivery its upload grants.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UploadGrantMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: UploadGrant,
}

/// Asks the Controller for fresh grants; the delivery keeps running while it waits.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UploadGrantNeededMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: UploadGrantNeeded,
}

impl ValidateMessage for DeliverRevisionMessage {
    /// Rejects inputs that could write outside the Revision namespace or the delivery's keys.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        self.payload.spec.validate()
    }
}

impl ValidateMessage for RevisionResultMessage {
    /// Applies delivery-owned terminal checks before event delivery or replay.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        self.payload.validate()
    }
}

impl ValidateMessage for UploadGrantMessage {
    /// Requires at least one usable grant; matching keys against the input is the Node's job.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        if self.payload.grants.is_empty() {
            return Err(MessageValidationError::EmptyUploadGrant);
        }
        self.payload
            .grants
            .iter()
            .try_for_each(ObjectUploadGrant::validate)
    }
}

impl ValidateMessage for UploadGrantNeededMessage {
    /// Requires the requesting Node and at least one object, each with a valid key and canonical
    /// digest; a delivery that needs a grant always has an object left to upload.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        validate_identity(self.payload.node_id.is_empty(), "node_id")?;
        if self.payload.checksums.is_empty() {
            return Err(MessageValidationError::EmptyUploadGrant);
        }
        self.payload.checksums.iter().try_for_each(|(key, digest)| {
            key.validate()?;
            digest.validate()
        })
    }
}
