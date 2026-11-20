//! Independent, value-free Agent/probe stop journal, sharing the Node's exclusive database lease.
mod audit;
mod migration;
mod model;
use crate::{AttemptId, Error, SessionJournal, WriteGuard, WritePoint};
pub(crate) use migration::{legacy_running, migrate};
pub use model::*;
use ora_node_protocol::{AgentSessionEnded, ExecutionId, StopContainment, StopUnconfirmed};
use rusqlite::{Connection, OptionalExtension, params};

/// Converts an enum through its canonical serde spelling, without a parallel token mapping.
fn token<T: serde::Serialize>(value: &T) -> Result<String, Error> {
    let value = serde_json::to_value(value)?;
    value
        .as_str()
        .map(str::to_owned)
        .ok_or(Error::InvalidTransition)
}

/// Reads exact identities before any stop operation acts on a lock or process clue.
pub(crate) fn records(
    connection: &Connection,
    execution: Option<&ExecutionId>,
) -> Result<Vec<SessionStopRecord>, Error> {
    let mut statement = connection.prepare("SELECT record,state,created_at_ms,first_stop_at_ms,bound_at_ms,confirmed_at_ms,resources_released,evidence,mark_scope FROM session_stop_attempts WHERE (?1 IS NULL OR execution=?1) ORDER BY created_at_ms,attempt")?;
    let rows = statement.query_map([execution.map(ExecutionId::as_str)], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, bool>(8)?,
        ))
    })?;
    rows.map(|row| {
        let (
            identity,
            state,
            created_at_ms,
            first_stop_at_ms,
            bound_at_ms,
            confirmed_at_ms,
            resources_released,
            proof,
            scoped,
        ) = row?;
        Ok(SessionStopRecord {
            identity: serde_json::from_str(&identity)?,
            state: serde_json::from_value(serde_json::Value::String(state))?,
            created_at_ms,
            first_stop_at_ms,
            bound_at_ms,
            confirmed_at_ms,
            resources_released,
            proof: proof
                .map(|proof| serde_json::from_value(serde_json::Value::String(proof)))
                .transpose()?,
            scope: if scoped {
                SessionStopScope::Recorded
            } else {
                SessionStopScope::Absent
            },
        })
    })
    .collect()
}

/// The terminal event and delivery may never outrun any Agent or probe responsibility.
pub(crate) fn pending(connection: &Connection, execution: &ExecutionId) -> Result<bool, Error> {
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM session_stop_attempts WHERE execution=?1 AND (state!='confirmed' OR resources_released=0))", [execution.as_str()], |r| r.get(0))?)
}

/// Preserves original stop identity and deadlines in execution-status replies.
pub(crate) fn unconfirmed(
    connection: &Connection,
    execution: &ExecutionId,
) -> Result<Vec<StopUnconfirmed>, Error> {
    records(connection, Some(execution))?
        .into_iter()
        .filter(|r| {
            matches!(
                r.state,
                SessionStopState::StopUnconfirmed | SessionStopState::LegacyUnknown
            )
        })
        .map(|r| {
            Ok(StopUnconfirmed {
                scope_id: r.identity.execution.as_str().to_owned(),
                run_id: r.identity.attempt.as_str().to_owned(),
                first_stop_at_ms: r.first_stop_at_ms.ok_or(Error::InvalidTransition)?,
                bound_at_ms: r.bound_at_ms.ok_or(Error::InvalidTransition)?,
                guarantee: StopContainment::BestEffort,
            })
        })
        .collect()
}

impl<G: WriteGuard> SessionJournal<G> {
    /// Status queries carry residual ownership even while the original actor is absent.
    pub fn session_stop_unconfirmed(
        &self,
        execution: &ExecutionId,
    ) -> Result<Vec<StopUnconfirmed>, Error> {
        unconfirmed(&self.lock()?.connection, execution)
    }
    /// Rejects rebinding and terminal executions; an identical retry is harmless.
    pub fn record_session_stop(
        &self,
        identity: &SessionStopIdentity,
        scope: SessionStopScope,
        at_ms: i64,
    ) -> Result<(), Error> {
        let mut db = self.lock()?;
        if identity.node.node_id != db.node_id
            || identity.packages.len() > 17
            || AttemptId::parse(identity.attempt.as_str()).as_ref() != Some(&identity.attempt)
        {
            return Err(Error::InvalidTransition);
        }
        let record = serde_json::to_string(identity)?;
        db.guard.before_write(WritePoint::Progress)?;
        let tx = db
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let scoped = scope == SessionStopScope::Recorded;
        let prior: Option<(String, bool)> = tx
            .query_row(
                "SELECT record,mark_scope FROM session_stop_attempts WHERE attempt=?1",
                [identity.attempt.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some(prior) = prior {
            return if prior == (record, scoped) {
                Ok(())
            } else {
                Err(Error::IdentityConflict)
            };
        }
        let allowed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM node_executions WHERE execution=?1 AND state='running' AND json_extract(input,'$.payload.spec.checkout_execution_id')=?2)", params![identity.execution.as_str(),identity.checkout.as_str()], |r| r.get(0))?;
        if !allowed {
            return Err(Error::InvalidTransition);
        }
        tx.execute("INSERT INTO session_stop_attempts(attempt,execution,checkout,record,state,created_at_ms,mark_scope) VALUES(?1,?2,?3,?4,'prepared',?5,?6)", params![identity.attempt.as_str(),identity.execution.as_str(),identity.checkout.as_str(),record,at_ms,scoped])?;
        tx.commit()?;
        Ok(())
    }

    /// Returns the responsibility set; missing or unreadable rows never become affirmative proof.
    pub fn session_stop_records(
        &self,
        execution: Option<&ExecutionId>,
    ) -> Result<Vec<SessionStopRecord>, Error> {
        records(&self.lock()?.connection, execution)
    }

    /// Anchors stop once, then confirms and releases in separate durable steps.
    pub fn change_session_stop(
        &self,
        attempt: &AttemptId,
        change: SessionStopChange,
    ) -> Result<(), Error> {
        let mut db = self.lock()?;
        db.guard.before_write(WritePoint::Progress)?;
        let tx = db
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        apply_change(&tx, attempt, &change)?;
        tx.commit()?;
        Ok(())
    }

    /// Keeps the first terminal intention while independent cleanup proceeds. It settles no
    /// cancel: only [`Self::decide_session_end`], which also anchors the stop clock, does.
    pub fn defer_session_stop_end(
        &self,
        execution: &ExecutionId,
        ended: &AgentSessionEnded,
    ) -> Result<(), Error> {
        let db = self.lock()?;
        db.guard.before_write(WritePoint::Progress)?;
        db.connection.execute(
            "INSERT OR IGNORE INTO session_stop_ends(execution,ended) VALUES(?1,?2)",
            params![execution.as_str(), serde_json::to_string(ended)?],
        )?;
        Ok(())
    }

    /// Records the end decision, anchors the stop clock of every outstanding stop
    /// responsibility of the execution, and settles every pending cancel as `session_ending`, all
    /// in one transaction, before any stop signal (cancel-turn D3). The first decision and the
    /// first clock are kept: a repeated call changes neither and settles nothing again.
    pub fn decide_session_end(
        &self,
        execution: &ExecutionId,
        ended: &AgentSessionEnded,
        at_ms: i64,
        bound_at_ms: i64,
    ) -> Result<(), Error> {
        if bound_at_ms <= at_ms {
            return Err(Error::InvalidTransition);
        }
        let mut db = self.lock()?;
        let guard = std::sync::Arc::clone(&db.guard);
        guard.before_write(WritePoint::Progress)?;
        let tx = db
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO session_stop_ends(execution,ended) VALUES(?1,?2)",
            params![execution.as_str(), serde_json::to_string(ended)?],
        )?;
        tx.execute("UPDATE session_stop_attempts SET first_stop_at_ms=coalesce(first_stop_at_ms,?2),bound_at_ms=coalesce(bound_at_ms,?3),state=CASE WHEN state='prepared' THEN 'stopping' ELSE state END WHERE execution=?1 AND state!='confirmed'", params![execution.as_str(), at_ms, bound_at_ms])?;
        // Settling is idempotent: only still-queued cancels are written.
        if let Some(current) = crate::session::read_running(&tx, execution)? {
            crate::session::settle_pending_as_ending(&tx, &guard, &current)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Restarts settle the original intent only after the stop owner clears every responsibility.
    pub fn session_stop_end(
        &self,
        execution: &ExecutionId,
    ) -> Result<Option<AgentSessionEnded>, Error> {
        let db = self.lock()?;
        let value: Option<String> = db
            .connection
            .query_row(
                "SELECT ended FROM session_stop_ends WHERE execution=?1",
                [execution.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(Error::from))
            .transpose()
    }
}

/// Applies only monotone changes, retaining the original deadline when stop is requested again.
fn apply_change(
    connection: &Connection,
    attempt: &AttemptId,
    change: &SessionStopChange,
) -> Result<(), Error> {
    let changed = match change {
        SessionStopChange::Request { at_ms, bound_at_ms } => {
            if bound_at_ms <= at_ms { return Err(Error::InvalidTransition); }
            connection.execute("UPDATE session_stop_attempts SET first_stop_at_ms=coalesce(first_stop_at_ms,?2),bound_at_ms=coalesce(bound_at_ms,?3),state=CASE WHEN state='prepared' THEN 'stopping' ELSE state END WHERE attempt=?1", params![attempt.as_str(),at_ms,bound_at_ms])?
        }
        SessionStopChange::Unconfirmed => connection.execute("UPDATE session_stop_attempts SET state=CASE WHEN state='legacy_unknown' THEN state ELSE 'stop_unconfirmed' END WHERE attempt=?1 AND state!='confirmed' AND first_stop_at_ms IS NOT NULL", [attempt.as_str()])?,
        SessionStopChange::Confirm { at_ms, proof } => connection.execute("UPDATE session_stop_attempts SET state='confirmed',confirmed_at_ms=coalesce(confirmed_at_ms,?2),evidence=coalesce(evidence,?3) WHERE attempt=?1 AND NOT (state='legacy_unknown' AND ?3 IN ('lock_and_processes','lock_processes_and_marks'))", params![attempt.as_str(),at_ms,token(proof)?])?,
        SessionStopChange::ResourcesReleased => connection.execute("UPDATE session_stop_attempts SET resources_released=1 WHERE attempt=?1 AND state='confirmed'", [attempt.as_str()])?,
    };
    if changed != 1 {
        return Err(Error::InvalidTransition);
    }
    Ok(())
}
