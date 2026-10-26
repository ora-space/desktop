use crate::*;
use ora_node_protocol::{ExecutionFence, ExecutionNonAcceptance, ValidateMessage};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

impl<G: WriteGuard> NodeDatabase<G> {
    /// Closes this original execution's intake in the same writer transaction that proves absence.
    /// Known acceptance is not an error and produces no proof; the caller keeps reconciling it.
    pub fn fence_unaccepted_execution(
        &mut self,
        fence: &ExecutionFence,
    ) -> Result<Option<ExecutionNonAcceptance>, Error> {
        fence.validate()?;
        let origin = &fence.origin;
        if fence.authority.node_id != self.node_id.as_str() {
            return Err(Error::NodeMismatch);
        }
        super::runtime_control::fresh_authority(&fence.authority)?;
        let bound = self.runtime_binding()?.ok_or(Error::InvalidTransition)?;
        if bound.tenant_id != fence.authority.tenant_id
            || bound.workspace_id != fence.authority.workspace_id
            || bound.sandbox_id != fence.authority.sandbox_id
            || bound.runtime_generation != fence.authority.runtime_generation
            || bound.node_incarnation_id != fence.authority.node_incarnation_id
            || bound.controller_epoch > fence.authority.controller_epoch
            || bound.control_epoch > fence.authority.control_epoch
        {
            return Err(Error::InvalidTransition);
        }
        self.guard.before_write(WritePoint::Accept)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT proof FROM execution_non_acceptance WHERE execution=?1 OR operation=?2",
                params![
                    origin.binding.execution_id,
                    origin.binding.node_operation_id
                ],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(data) = existing {
            let proof: ExecutionNonAcceptance = serde_json::from_str(&data)?;
            if proof.fence.origin != *origin {
                return Err(Error::IdentityConflict);
            }
            // An old proof can survive an incarnation change; a new empty ledger cannot create it.
            return Ok(Some(proof));
        }
        if origin.binding.node_instance_id != fence.authority.node_instance_id
            || origin.binding.node_incarnation_id != fence.authority.node_incarnation_id
        {
            return Ok(None);
        }
        let known: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM execution_identities WHERE execution=?1 OR operation=?2)
             OR EXISTS(SELECT 1 FROM process_attempts WHERE execution=?1)
             OR EXISTS(SELECT 1 FROM session_restores WHERE execution=?1)",
            params![
                origin.binding.execution_id,
                origin.binding.node_operation_id
            ],
            |row| row.get(0),
        )?;
        if known {
            return Ok(None);
        }
        let proof = ExecutionNonAcceptance {
            proof_id: uuid::Uuid::new_v4().to_string(),
            fence: fence.clone(),
        };
        tx.execute(
            "INSERT INTO execution_non_acceptance VALUES(?1,?2,?3)",
            params![
                origin.binding.execution_id,
                origin.binding.node_operation_id,
                serde_json::to_string(&proof)?
            ],
        )?;
        tx.commit()?;
        Ok(Some(proof))
    }
}
