use super::{CloneRepositoryMessage, MessageValidationError, ValidateMessage};
use serde::{Deserialize, Serialize};

/// Cloud-issued scope. User control, Controller lease, runtime and host identities remain separate.
/// This extension requires the negotiated RuntimeControl capability; legacy commands carry none.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeBinding {
    pub tenant_id: String,
    pub workspace_id: String,
    pub sandbox_id: String,
    pub runtime_generation: i64,
    pub node_id: String,
    pub node_incarnation_id: String,
    pub node_instance_id: String,
    pub controller_epoch: i64,
    pub control_epoch: i64,
    pub control_version: i64,
    pub session_id: String,
    pub actor_user_id: String,
    pub operation_id: String,
    pub execution_id: String,
    /// Node-local immutable operation for this attempt; operation_id is the Cloud business intent.
    #[serde(default)]
    pub node_operation_id: String,
    pub input_closed: bool,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

impl ValidateMessage for RuntimeBinding {
    fn validate(&self) -> Result<(), MessageValidationError> {
        for (field, value) in [
            ("tenant_id", &self.tenant_id),
            ("workspace_id", &self.workspace_id),
            ("sandbox_id", &self.sandbox_id),
            ("node_id", &self.node_id),
            ("node_incarnation_id", &self.node_incarnation_id),
            ("node_instance_id", &self.node_instance_id),
        ] {
            if value.trim().is_empty() || value.len() > 256 {
                return Err(MessageValidationError::EmptyField { field });
            }
        }
        if self.controller_epoch <= 0
            || self.control_epoch < 0
            || self.control_version <= 0
            || self.runtime_generation <= 0
            || self.issued_at_ms <= 0
            || (!self.input_closed
                && (self.control_epoch == 0
                    || self.expires_at_ms <= self.issued_at_ms
                    || self.expires_at_ms - self.issued_at_ms > 60_000
                    || (self.session_id.is_empty() == self.operation_id.is_empty())))
        {
            return Err(MessageValidationError::InvalidRuntimeBinding);
        }
        Ok(())
    }
}

/// A fresh dispatch permit for exactly one already registered execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlledClone {
    pub binding: RuntimeBinding,
    pub command: CloneRepositoryMessage,
}

/// Plugin execution with the same fresh, exact runtime authority required by clones.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlledPlugins {
    pub binding: RuntimeBinding,
    pub command: super::PluginCommand,
}

impl ValidateMessage for ControlledPlugins {
    /// Refuses a permit for another execution, operation or target before admission.
    fn validate(&self) -> Result<(), MessageValidationError> {
        self.binding.validate()?;
        self.command.validate()?;
        if self.binding.input_closed
            || self.binding.execution_id != self.command.execution_id().as_str()
            || self.binding.node_operation_id != self.command.operation_id().as_str()
            || self.binding.node_id != self.command.node_id().as_str()
        {
            return Err(MessageValidationError::InvalidRuntimeBinding);
        }
        Ok(())
    }
}

impl ValidateMessage for ControlledClone {
    fn validate(&self) -> Result<(), MessageValidationError> {
        self.binding.validate()?;
        self.command.validate()?;
        if self.binding.input_closed
            || self.binding.execution_id != self.command.execution_id.as_str()
            || self.binding.node_operation_id != self.command.operation_id.as_str()
            || self.binding.node_id != self.command.payload.spec.node_id.as_str()
        {
            return Err(MessageValidationError::InvalidRuntimeBinding);
        }
        Ok(())
    }
}

/// Durable input-closure acknowledgement plus unresolved liabilities. Empty is an observed fact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeControlState {
    pub binding: RuntimeBinding,
    pub unfinished_execution_ids: Vec<String>,
}

impl ValidateMessage for RuntimeControlState {
    fn validate(&self) -> Result<(), MessageValidationError> {
        self.binding.validate()?;
        if self
            .unfinished_execution_ids
            .iter()
            .any(|v| v.is_empty() || v.len() > 256)
        {
            return Err(MessageValidationError::InvalidRuntimeBinding);
        }
        Ok(())
    }
}

/// Session start with the same exact runtime permit required by clone and plugin execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlledStartAgentSession {
    pub binding: RuntimeBinding,
    pub command: super::StartAgentSessionMessage,
}

impl ValidateMessage for ControlledStartAgentSession {
    /// Rejects cross-execution or cross-Node authority before durable admission.
    fn validate(&self) -> Result<(), MessageValidationError> {
        self.binding.validate()?;
        self.command.validate()?;
        if self.binding.input_closed
            || self.binding.execution_id != self.command.execution_id.as_str()
            || self.binding.node_operation_id != self.command.operation_id.as_str()
            || self.binding.node_id != self.command.payload.spec.node_id.as_str()
        {
            return Err(MessageValidationError::InvalidRuntimeBinding);
        }
        Ok(())
    }
}

/// Revision delivery with the same exact runtime permit required by clone, plugin and session
/// execution, so a delivery cannot start on a Node or Workspace runtime the Cloud has not bound.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlledDeliverRevision {
    pub binding: RuntimeBinding,
    pub command: super::DeliverRevisionMessage,
}

impl ValidateMessage for ControlledDeliverRevision {
    /// Rejects cross-execution or cross-Node authority before durable admission.
    fn validate(&self) -> Result<(), MessageValidationError> {
        self.binding.validate()?;
        self.command.validate()?;
        if self.binding.input_closed
            || self.binding.execution_id != self.command.execution_id.as_str()
            || self.binding.node_operation_id != self.command.operation_id.as_str()
            || self.binding.node_id != self.command.payload.spec.node_id.as_str()
        {
            return Err(MessageValidationError::InvalidRuntimeBinding);
        }
        Ok(())
    }
}
