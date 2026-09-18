use ora_process_protocol::{RunId, ScopeId};
use rusqlite::Connection;

use super::{HostState, ProcessStateError};

pub(super) const STOP_SCHEMA: &str = "CREATE TABLE run_stop_intents (
    run TEXT PRIMARY KEY NOT NULL REFERENCES run_intents(run)
) STRICT";
pub(super) const CLOSE_SCHEMA: &str = "CREATE TABLE scope_close_intents (
    scope TEXT PRIMARY KEY NOT NULL REFERENCES scope_intents(scope)
) STRICT";

impl HostState {
    /// Accepts irreversible force-stop responsibility, independently of guardian connectivity.
    pub fn request_run_stop(&mut self, run: RunId) -> Result<(), ProcessStateError> {
        self.run_intent(run)?
            .ok_or(ProcessStateError::Rejected("unknown host Run"))?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO run_stop_intents VALUES (?1)",
            [run.to_string()],
        )?;
        transaction.commit()?;
        self.layout.sync()?;
        Ok(())
    }

    /// Seals host admission before acknowledging close; a duplicate never reopens the Scope.
    pub fn request_scope_close(&mut self, scope: ScopeId) -> Result<(), ProcessStateError> {
        self.scope_intent(scope)?
            .ok_or(ProcessStateError::Rejected("unknown host Scope"))?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO scope_close_intents VALUES (?1)",
            [scope.to_string()],
        )?;
        transaction.commit()?;
        self.layout.sync()?;
        Ok(())
    }

    /// Includes the enclosing Scope's close intention without rewriting each Run's immutable intent.
    pub fn run_stop_requested(&self, run: RunId) -> Result<bool, ProcessStateError> {
        let intent = self
            .run_intent(run)?
            .ok_or(ProcessStateError::Rejected("unknown host Run"))?;
        Ok(self.scope_close_requested(intent.scope)?
            || self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM run_stop_intents WHERE run=?1)",
                [run.to_string()],
                |row| row.get(/*idx*/ 0),
            )?)
    }

    /// Reports intent only, not guardian admission sealing or completed cleanup.
    pub fn scope_close_requested(&self, scope: ScopeId) -> Result<bool, ProcessStateError> {
        if !self.connection.is_autocommit() {
            return Err(ProcessStateError::Rejected(
                "host journal has an unfinished transaction",
            ));
        }
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM scope_close_intents WHERE scope=?1)",
            [scope.to_string()],
            |row| row.get(/*idx*/ 0),
        )?)
    }

    /// Enumerates original Scope responsibilities, including empty scopes that still require close.
    pub fn scope_intents(
        &self,
    ) -> Result<Vec<ora_process_protocol::ScopeCreationIntent>, ProcessStateError> {
        let mut statement = self
            .connection
            .prepare("SELECT scope FROM scope_intents ORDER BY scope")?;
        let scopes = statement.query_map([], |row| row.get::<_, String>(/*idx*/ 0))?;
        let mut intents = Vec::new();
        for scope in scopes {
            let scope = scope?.parse()?;
            intents.push(
                self.scope_intent(scope)?
                    .ok_or(ProcessStateError::Rejected("missing Scope intent"))?,
            );
        }
        Ok(intents)
    }
}

/// Validates references before advancing authority, even if an external writer disabled foreign keys.
pub(super) fn inspect(connection: &Connection) -> Result<(), ProcessStateError> {
    let orphaned: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM run_stop_intents WHERE run NOT IN (SELECT run FROM run_intents)) OR EXISTS(SELECT 1 FROM scope_close_intents WHERE scope NOT IN (SELECT scope FROM scope_intents))",
        [], |row| row.get(/*idx*/ 0),
    )?;
    if orphaned {
        return Err(ProcessStateError::Rejected("orphaned host control intent"));
    }
    Ok(())
}
