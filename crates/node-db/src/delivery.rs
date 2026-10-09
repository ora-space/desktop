//! Bounded replay pages selected after the current connection's reserved sequences.
use crate::{Error, NodeDatabase, WriteGuard};
use ora_node_protocol::{ControllerId, ExecutionId, NodeToControllerMessage};
use serde::Serialize;

/// A connection-local cursor, never durable acknowledgement or execution authority.
#[derive(Clone, Debug, Serialize)]
pub struct EventCursor {
    pub execution: ExecutionId,
    pub after: u64,
    pub remaining: usize,
}

impl<G: WriteGuard> NodeDatabase<G> {
    /// Reads a bounded page without repeatedly selecting a full execution's oldest events.
    /// The transport reserves each returned event against its window before sending it.
    pub fn controller_events_after(
        &self,
        controller: &ControllerId,
        cursors: &[EventCursor],
    ) -> Result<Vec<NodeToControllerMessage>, Error> {
        let mut statement = self.connection.prepare(
            "WITH cursors AS (SELECT json_extract(value,'$.execution') AS execution, json_extract(value,'$.after') AS sent, json_extract(value,'$.remaining') AS remaining FROM json_each(?2)), events AS (SELECT execution,1 AS sequence,event FROM clone_outbox UNION ALL SELECT execution,1 AS sequence,event FROM plugin_outbox UNION ALL SELECT execution,1 AS sequence,event FROM revision_outbox UNION ALL SELECT execution,sequence,event FROM execution_events) SELECT event FROM events JOIN execution_controllers USING(execution) LEFT JOIN cursors USING(execution) WHERE controller=?1 AND sequence>COALESCE(sent,0) AND COALESCE(remaining,256)>0 ORDER BY execution,sequence LIMIT 16"
        )?;
        statement
            .query_map(
                rusqlite::params![controller.as_str(), serde_json::to_string(cursors)?],
                |row| row.get::<_, String>(/*idx*/ 0),
            )?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect()
    }
}
