use super::{MessageValidationError, RuntimeBinding, ValidateMessage};
use serde::{Deserialize, Serialize};

/// Original dispatch identity, including immutable work input rather than a current empty ledger.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionOrigin {
    pub binding: RuntimeBinding,
    pub work_id: String,
    pub input_json: String,
}

/// Current authority to fence exactly one administratively settled execution's intake.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionFence {
    pub authority: RuntimeBinding,
    pub origin: ExecutionOrigin,
}

/// A durable refusal of future admission after an atomic check found no original acceptance.
/// It carries no ordinary execution sequence and never authorizes an event ACK.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionNonAcceptance {
    pub proof_id: String,
    pub fence: ExecutionFence,
}

impl ValidateMessage for ExecutionFence {
    /// Keeps resource scope and frozen original identity distinct from the current control lease.
    fn validate(&self) -> Result<(), MessageValidationError> {
        self.authority.validate()?;
        let original = &self.origin.binding;
        original.validate()?;
        let input: serde_json::Value = serde_json::from_str(&self.origin.input_json)
            .map_err(|_| MessageValidationError::InvalidRuntimeBinding)?;
        if self.origin.work_id.is_empty()
            || self.origin.work_id.len() > 256
            || original.execution_id.is_empty()
            || original.node_operation_id.is_empty()
            || !input.is_object()
            || !matches!(
                input.get("kind").and_then(serde_json::Value::as_str),
                Some("deliver_revision" | "deliver_scheduled_artifacts")
            )
            || self.authority.tenant_id != original.tenant_id
            || self.authority.workspace_id != original.workspace_id
            || self.authority.sandbox_id != original.sandbox_id
            || self.authority.runtime_generation != original.runtime_generation
            || self.authority.node_id != original.node_id
        {
            return Err(MessageValidationError::InvalidRuntimeBinding);
        }
        Ok(())
    }
}

impl ValidateMessage for ExecutionNonAcceptance {
    /// Historical proof replay validates identity without renewing its old authority timestamps.
    fn validate(&self) -> Result<(), MessageValidationError> {
        self.fence.validate()?;
        if self.proof_id.trim().is_empty() || self.proof_id.len() > 256 {
            return Err(MessageValidationError::InvalidRuntimeBinding);
        }
        Ok(())
    }
}
