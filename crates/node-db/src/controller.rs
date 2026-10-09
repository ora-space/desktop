use crate::*;
use ora_node_protocol::{ControllerId, ExecutionId, OperationId};
use rusqlite::OptionalExtension;

impl<G: WriteGuard> NodeDatabase<G> {
    /// Persists deployment ownership, never letting a handshake adopt historical executions.
    pub fn bind_controller(&mut self, controller: &ControllerId) -> Result<(), Error> {
        if controller.as_str().trim().is_empty() {
            return Err(Error::ControllerMismatch);
        }
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT controller FROM controller_binding WHERE singleton=1",
                [],
                |row| row.get(/*idx*/ 0),
            )
            .optional()?;
        if let Some(existing) = existing {
            return if existing == controller.as_str() {
                Ok(())
            } else {
                Err(Error::ControllerMismatch)
            };
        }
        self.guard.before_write(WritePoint::Accept)?;
        self.connection.execute(
            "INSERT INTO controller_binding VALUES (1,?1)",
            [controller.as_str()],
        )?;
        Ok(())
    }

    /// Allows new identities or this owner's existing execution, but refuses legacy or foreign responsibility.
    pub fn check_controller_execution(
        &self,
        controller: &ControllerId,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<(), Error> {
        let bound: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM controller_binding WHERE controller=?1)",
            [controller.as_str()],
            |row| row.get(/*idx*/ 0),
        )?;
        if !bound {
            return Err(Error::ControllerMismatch);
        }
        if self.identity_kind(operation, execution)?.is_none() {
            return Ok(());
        }
        let owned: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM execution_controllers WHERE execution=?1 AND controller=?2)",
            rusqlite::params![execution.as_str(), controller.as_str()], |row| row.get(/*idx*/ 0),
        )?;
        if owned {
            Ok(())
        } else {
            Err(Error::ControllerMismatch)
        }
    }

    /// Replays only deliveries attributed at acceptance, leaving unclaimed history untouched.
    pub fn controller_events(
        &self,
        controller: &ControllerId,
    ) -> Result<Vec<ora_node_protocol::NodeToControllerMessage>, Error> {
        let mut statement = self.connection.prepare(
            "SELECT event FROM (SELECT execution,1 AS sequence,event FROM clone_outbox UNION ALL SELECT execution,1 AS sequence,event FROM plugin_outbox UNION ALL SELECT execution,1 AS sequence,event FROM revision_outbox UNION ALL SELECT execution,sequence,event FROM execution_events) JOIN execution_controllers USING(execution) WHERE controller=?1 ORDER BY execution,sequence LIMIT 16",
        )?;
        statement
            .query_map([controller.as_str()], |row| {
                row.get::<_, String>(/*idx*/ 0)
            })?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect()
    }
}
