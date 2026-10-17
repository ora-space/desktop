//! The Node's ledger of own Workspace branch values (Node clone ADR D6–D8) and the checkout
//! facts `ora-git`, session and delivery read from successful clone evidence.
use crate::*;
use ora_node_protocol::*;
use rusqlite::{OptionalExtension, params};

impl<G: WriteGuard> NodeDatabase<G> {
    /// The checkout a successful clone execution left, with the input it was made from.
    pub fn cloned_checkout(
        &self,
        execution: &ExecutionId,
    ) -> Result<Option<ClonedCheckout>, Error> {
        let row: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT target,progress FROM clone_executions WHERE execution=?1",
                [execution.as_str()],
                |r| Ok((r.get(/*idx*/ 0)?, r.get(/*idx*/ 1)?)),
            )
            .optional()?;
        let Some((target, progress)) = row else {
            return Ok(None);
        };
        match serde_json::from_str::<CloneProgress>(&progress)? {
            CloneProgress::Completed(result) => match *result {
                CloneExecutionResult::CloneReady(ready) => Ok(Some(ClonedCheckout {
                    path: serde_json::from_str::<CloneTarget>(&target)?.path,
                    spec: ready.spec,
                })),
                CloneExecutionResult::CloneFailed(_) => Ok(None),
            },
            CloneProgress::Pending(_) | CloneProgress::Unknown(_) => Ok(None),
        }
    }

    /// The last known value of `branch` for `checkout`, if any push or fetch recorded one.
    pub fn branch_head(
        &self,
        checkout: &ExecutionId,
        branch: &WorkspaceRef,
    ) -> Result<Option<CommitId>, Error> {
        Ok(self
            .connection
            .query_row(
                "SELECT commit_id FROM workspace_branch_heads WHERE checkout=?1 AND ref=?2",
                params![checkout.as_str(), branch.as_str()],
                |r| r.get::<_, String>(/*idx*/ 0),
            )
            .optional()?
            .map(CommitId::new))
    }

    /// Records the value `branch` is now known to have on the Git Server.
    pub fn record_branch_head(
        &mut self,
        checkout: &ExecutionId,
        branch: &WorkspaceRef,
        commit: &CommitId,
    ) -> Result<(), Error> {
        self.guard.before_write(WritePoint::Progress)?;
        self.connection.execute(
            "INSERT INTO workspace_branch_heads VALUES (?1,?2,?3) ON CONFLICT(checkout,ref) DO UPDATE SET commit_id=excluded.commit_id",
            params![checkout.as_str(), branch.as_str(), commit.as_str()],
        )?;
        Ok(())
    }

    /// Every own branch value recorded for `checkout`, for fetch negotiation.
    pub fn branch_heads(&self, checkout: &ExecutionId) -> Result<Vec<CommitId>, Error> {
        self.connection
            .prepare("SELECT commit_id FROM workspace_branch_heads WHERE checkout=?1 ORDER BY ref")?
            .query_map([checkout.as_str()], |r| r.get::<_, String>(/*idx*/ 0))?
            .map(|commit| Ok(CommitId::new(commit?)))
            .collect()
    }
}

impl<G: WriteGuard> SessionJournal<G> {
    /// See [`NodeDatabase::cloned_checkout`].
    pub fn cloned_checkout(
        &self,
        execution: &ExecutionId,
    ) -> Result<Option<ClonedCheckout>, Error> {
        self.lock()?.cloned_checkout(execution)
    }

    /// See [`NodeDatabase::branch_head`].
    pub fn branch_head(
        &self,
        checkout: &ExecutionId,
        branch: &WorkspaceRef,
    ) -> Result<Option<CommitId>, Error> {
        self.lock()?.branch_head(checkout, branch)
    }

    /// See [`NodeDatabase::record_branch_head`].
    pub fn record_branch_head(
        &self,
        checkout: &ExecutionId,
        branch: &WorkspaceRef,
        commit: &CommitId,
    ) -> Result<(), Error> {
        self.lock()?.record_branch_head(checkout, branch, commit)
    }

    /// See [`NodeDatabase::branch_heads`].
    pub fn branch_heads(&self, checkout: &ExecutionId) -> Result<Vec<CommitId>, Error> {
        self.lock()?.branch_heads(checkout)
    }
}
