use ora_process_protocol::{
    GUARDIAN_WIRE_VERSION, GuardianHostSession, GuardianRunRequest, HostRunIntent, RunId,
    decode_guardian_payload, encode_guardian_frame,
};
use rusqlite::{Connection, params};

use super::{HostState, ProcessStateError};

pub(super) const SCHEMA: &str = "CREATE TABLE run_intents (
    run TEXT PRIMARY KEY NOT NULL,
    scope TEXT NOT NULL REFERENCES scope_intents(scope),
    intent BLOB NOT NULL
) STRICT";

impl HostState {
    /// Records an immutable responsibility before dispatch; a lost acknowledgement is queried by RunId.
    /// This call neither starts a guardian nor proves that the Run exists there.
    pub fn record_run_intent(
        &mut self,
        intent: HostRunIntent,
    ) -> Result<HostRunIntent, ProcessStateError> {
        if let Some(existing) = self.run_intent(intent.run)? {
            if existing != intent {
                return Err(ProcessStateError::Rejected(
                    "RunId already has a different intent",
                ));
            }
            self.layout.sync()?;
            return Ok(existing);
        }
        let scope = self
            .scope_intent(intent.scope)?
            .ok_or(ProcessStateError::Rejected(
                "Run intent requires an existing scope intent",
            ))?;
        if self.scope_close_requested(intent.scope)? {
            return Err(ProcessStateError::Rejected("host Scope is closing"));
        }
        // Admission must fit the actual outgoing message, not just its smaller stored payload.
        let operation = intent.start_operation();
        encode_guardian_frame(&GuardianRunRequest {
            version: GUARDIAN_WIRE_VERSION,
            intent: scope,
            channel: operation.channel(),
            session: GuardianHostSession { host: self.binding },
            operation,
        })?;
        let bytes = encode_guardian_frame(&intent)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO run_intents VALUES (?1, ?2, ?3)",
            params![
                intent.run.to_string(),
                intent.scope.to_string(),
                &bytes[4..]
            ],
        )?;
        transaction.commit()?;
        self.layout.sync()?;
        Ok(intent)
    }

    /// Returns only host responsibility; absence is not proof that an older guardian never executed a Run.
    pub fn run_intent(&self, run: RunId) -> Result<Option<HostRunIntent>, ProcessStateError> {
        if !self.connection.is_autocommit() {
            return Err(ProcessStateError::Rejected(
                "host journal has an unfinished transaction",
            ));
        }
        let mut statement = self
            .connection
            .prepare("SELECT run, scope, intent FROM run_intents WHERE run=?1")?;
        let mut rows = statement.query([run.to_string()])?;
        rows.next()?.map(decode).transpose()
    }

    /// Enumerates recorded responsibilities in stable identity order after caller memory is lost.
    /// No guardian journal is opened and no launch or binding is inferred from these records.
    pub fn run_intents(&self) -> Result<Vec<HostRunIntent>, ProcessStateError> {
        if !self.connection.is_autocommit() {
            return Err(ProcessStateError::Rejected(
                "host journal has an unfinished transaction",
            ));
        }
        let mut statement = self
            .connection
            .prepare("SELECT run, scope, intent FROM run_intents ORDER BY run")?;
        let mut rows = statement.query([])?;
        let mut intents = Vec::new();
        while let Some(row) = rows.next()? {
            intents.push(decode(row)?);
        }
        Ok(intents)
    }
}

/// Validates every indexed payload and foreign scope before host recovery can advance authority.
pub(super) fn inspect(connection: &Connection) -> Result<(), ProcessStateError> {
    let mut statement = connection.prepare("SELECT run, scope, intent FROM run_intents")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let intent = decode(row)?;
        if super::journal::find_intent(connection, intent.scope)?.is_none() {
            return Err(ProcessStateError::Rejected(
                "Run intent has no scope responsibility",
            ));
        }
    }
    Ok(())
}

/// The searchable columns are indexes of the typed payload, never an independent source of truth.
fn decode(row: &rusqlite::Row<'_>) -> Result<HostRunIntent, ProcessStateError> {
    let run: RunId = row.get::<_, String>(/*idx*/ 0)?.parse()?;
    let scope = row.get::<_, String>(/*idx*/ 1)?.parse()?;
    let bytes: Vec<u8> = row.get(/*idx*/ 2)?;
    let intent: HostRunIntent = decode_guardian_payload(&bytes)?;
    if intent.run != run || intent.scope != scope {
        return Err(ProcessStateError::Rejected(
            "Run intent index does not match its payload",
        ));
    }
    Ok(intent)
}
