use ora_process_protocol::{
    RunId, RunSnapshot, ScopeId, ScopeState, decode_guardian_payload, encode_guardian_frame,
};
use rusqlite::{Connection, OptionalExtension};

use super::{HostState, ProcessStateError};

pub(super) const RUN_SCHEMA: &str = "CREATE TABLE run_observations (
    run TEXT PRIMARY KEY NOT NULL REFERENCES run_intents(run), snapshot BLOB NOT NULL
) STRICT";
pub(super) const SCOPE_SCHEMA: &str = "CREATE TABLE scope_observations (
    scope TEXT PRIMARY KEY NOT NULL REFERENCES scope_intents(scope), snapshot BLOB NOT NULL
) STRICT";

impl HostState {
    /// Returns historical evidence only; recovery never upgrades it into a current live handle.
    pub(crate) fn observed_run(
        &self,
        run: RunId,
    ) -> Result<Option<RunSnapshot>, ProcessStateError> {
        let bytes: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT snapshot FROM run_observations WHERE run=?1",
                [run.to_string()],
                |row| row.get(/*idx*/ 0),
            )
            .optional()?;
        bytes
            .map(|bytes| decode_guardian_payload(&bytes).map_err(Into::into))
            .transpose()
    }

    /// Exposes the last durable Scope observation, separately from requested closure.
    pub(crate) fn observed_scope(
        &self,
        scope: ScopeId,
    ) -> Result<Option<ScopeState>, ProcessStateError> {
        let bytes: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT snapshot FROM scope_observations WHERE scope=?1",
                [scope.to_string()],
                |row| row.get(/*idx*/ 0),
            )
            .optional()?;
        bytes
            .map(|bytes| decode_guardian_payload(&bytes).map_err(Into::into))
            .transpose()
    }

    /// Commits a changed fact before host queries can expose it; unchanged polling does not rewrite it.
    pub(crate) fn observe_run(&mut self, snapshot: &RunSnapshot) -> Result<(), ProcessStateError> {
        if self.observed_run(snapshot.id)?.as_ref() != Some(snapshot) {
            let bytes = encode_guardian_frame(snapshot)?;
            self.connection.execute("INSERT INTO run_observations VALUES (?1, ?2) ON CONFLICT(run) DO UPDATE SET snapshot=excluded.snapshot", rusqlite::params![snapshot.id.to_string(), &bytes[4..]])?;
        }
        self.layout.sync()?;
        Ok(())
    }

    /// Retains closure evidence even when the original guardian is later unavailable.
    pub(crate) fn observe_scope(
        &mut self,
        scope: ScopeId,
        state: ScopeState,
    ) -> Result<(), ProcessStateError> {
        if self.observed_scope(scope)? != Some(state) {
            let bytes = encode_guardian_frame(&state)?;
            self.connection.execute("INSERT INTO scope_observations VALUES (?1, ?2) ON CONFLICT(scope) DO UPDATE SET snapshot=excluded.snapshot", rusqlite::params![scope.to_string(), &bytes[4..]])?;
        }
        self.layout.sync()?;
        Ok(())
    }
}

/// Checks indexes, typed facts and references read-only before any authority advancement.
pub(super) fn inspect(connection: &Connection) -> Result<(), ProcessStateError> {
    let mut statement = connection.prepare("SELECT run, snapshot FROM run_observations")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let run: RunId = row.get::<_, String>(/*idx*/ 0)?.parse()?;
        let snapshot: RunSnapshot = decode_guardian_payload(&row.get::<_, Vec<u8>>(/*idx*/ 1)?)?;
        if run != snapshot.id {
            return Err(ProcessStateError::Rejected(
                "Run observation identity mismatch",
            ));
        }
    }
    let mut statement = connection.prepare("SELECT scope, snapshot FROM scope_observations")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let _: ScopeId = row.get::<_, String>(/*idx*/ 0)?.parse()?;
        let _: ScopeState = decode_guardian_payload(&row.get::<_, Vec<u8>>(/*idx*/ 1)?)?;
    }
    let orphaned: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM run_observations WHERE run NOT IN (SELECT run FROM run_intents)) OR EXISTS(SELECT 1 FROM scope_observations WHERE scope NOT IN (SELECT scope FROM scope_intents))", [], |row| row.get(/*idx*/ 0))?;
    if orphaned {
        return Err(ProcessStateError::Rejected("orphaned host observation"));
    }
    Ok(())
}
