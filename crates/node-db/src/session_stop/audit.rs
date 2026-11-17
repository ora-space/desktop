//! Request-before-action and result-before-release management audit transactions.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct AuditInput {
    request: SessionStopReconcile,
    original: SessionStopIdentity,
}

impl<G: WriteGuard> SessionJournal<G> {
    /// Only a peer-credential UID may be passed here. The socket owner rejects non-root clients.
    pub fn begin_session_stop_audit(
        &self,
        actor_uid: u32,
        request: &SessionStopReconcile,
        at_ms: i64,
    ) -> Result<SessionStopAuditAdmission, Error> {
        if actor_uid != 0
            || request.request_id.trim().is_empty()
            || request.request_id.len() > 128
            || AttemptId::parse(request.attempt.as_str()).as_ref() != Some(&request.attempt)
        {
            return Err(Error::InvalidTransition);
        }
        let mut db = self.lock()?;
        db.guard.before_write(WritePoint::Progress)?;
        let tx = db
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT request,result FROM session_stop_audit WHERE request_id=?1",
                [&request.request_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((previous, result)) = previous {
            let previous: AuditInput = serde_json::from_str(&previous)?;
            if previous.request != *request {
                return Err(Error::IdentityConflict);
            }
            return Ok(match result {
                Some(result) => {
                    SessionStopAuditAdmission::Completed(serde_json::from_str(&result)?)
                }
                None => SessionStopAuditAdmission::InProgress,
            });
        }
        let record: String = tx
            .query_row(
                "SELECT record FROM session_stop_attempts WHERE attempt=?1 AND execution=?2",
                params![request.attempt.as_str(), request.execution.as_str()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::InvalidTransition)?;
        let input = AuditInput {
            request: request.clone(),
            original: serde_json::from_str(&record)?,
        };
        tx.execute("INSERT INTO session_stop_audit(request_id,execution,attempt,actor_uid,request,requested_at_ms) VALUES(?1,?2,?3,?4,?5,?6)",params![request.request_id,request.execution.as_str(),request.attempt.as_str(),actor_uid,serde_json::to_string(&input)?,at_ms])?;
        tx.commit()?;
        Ok(SessionStopAuditAdmission::Started)
    }

    /// A crash after request logging requires new evidence; the old request itself proves nothing.
    pub fn incomplete_session_stop_audits(&self) -> Result<Vec<SessionStopReconcile>, Error> {
        let db = self.lock()?;
        let mut statement = db.connection.prepare("SELECT request FROM session_stop_audit WHERE result IS NULL ORDER BY requested_at_ms LIMIT 1000")?;
        let rows = statement.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str::<AuditInput>(&r?)?.request))
            .collect()
    }

    /// Confirmation and the audit result commit together; a failed audit cannot clear a fence.
    pub fn finish_session_stop_audit(
        &self,
        request: &SessionStopReconcile,
        result: &SessionStopAuditResult,
        at_ms: i64,
    ) -> Result<(), Error> {
        if (result.observation == SessionStopObservation::Stopped) != result.proof.is_some()
            || (result.after == SessionStopState::Confirmed) != result.proof.is_some()
        {
            return Err(Error::InvalidTransition);
        }
        let mut db = self.lock()?;
        db.guard.before_write(WritePoint::Progress)?;
        let tx = db
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (input, prior): (String, Option<String>) = tx.query_row(
            "SELECT request,result FROM session_stop_audit WHERE request_id=?1",
            [&request.request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if serde_json::from_str::<AuditInput>(&input)?.request != *request {
            return Err(Error::IdentityConflict);
        }
        let encoded = serde_json::to_string(result)?;
        if let Some(prior) = prior {
            return if prior == encoded {
                Ok(())
            } else {
                Err(Error::IdentityConflict)
            };
        }
        if let Some(proof) = result.proof {
            apply_change(
                &tx,
                &request.attempt,
                &SessionStopChange::Confirm { at_ms, proof },
            )?;
        }
        tx.execute(
            "UPDATE session_stop_audit SET result=?2,completed_at_ms=?3 WHERE request_id=?1",
            params![request.request_id, encoded, at_ms],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Selects at most 1,000 fully released details after seven days. Unconfirmed rows never age out.
    pub fn session_stop_gc_candidates(&self, now_ms: i64) -> Result<Vec<AttemptId>, Error> {
        let db = self.lock()?;
        let mut statement = db.connection.prepare("SELECT attempt FROM session_stop_attempts a WHERE state='confirmed' AND resources_released=1 AND confirmed_at_ms<?1 AND NOT EXISTS(SELECT 1 FROM execution_events e WHERE e.execution=a.execution) AND NOT EXISTS(SELECT 1 FROM session_stop_audit u WHERE u.attempt=a.attempt AND u.result IS NULL) AND NOT EXISTS(SELECT 1 FROM session_stop_ends e JOIN node_executions n USING(execution) WHERE e.execution=a.execution AND n.state!='completed') ORDER BY confirmed_at_ms LIMIT 1000")?;
        let rows = statement.query_map([now_ms.saturating_sub(7 * 24 * 60 * 60 * 1000)], |r| {
            r.get::<_, String>(0)
        })?;
        rows.map(|r| AttemptId::parse(&r?).ok_or(Error::InvalidSchema))
            .collect()
    }

    /// Called after the confirmed detail's files are removed; the transaction rechecks references.
    pub fn retire_session_stop(&self, attempt: &AttemptId, now_ms: i64) -> Result<(), Error> {
        let mut db = self.lock()?;
        db.guard.before_write(WritePoint::Progress)?;
        let tx = db
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let candidate: Option<(String,i64)> = tx.query_row("SELECT execution,confirmed_at_ms FROM session_stop_attempts a WHERE attempt=?1 AND state='confirmed' AND resources_released=1 AND confirmed_at_ms<?2 AND NOT EXISTS(SELECT 1 FROM execution_events e WHERE e.execution=a.execution) AND NOT EXISTS(SELECT 1 FROM session_stop_audit u WHERE u.attempt=a.attempt AND u.result IS NULL) AND NOT EXISTS(SELECT 1 FROM session_stop_ends e JOIN node_executions n USING(execution) WHERE e.execution=a.execution AND n.state!='completed')", params![attempt.as_str(),now_ms.saturating_sub(7*24*60*60*1000)], |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (execution, confirmed) = candidate.ok_or(Error::InvalidTransition)?;
        tx.execute("INSERT INTO session_stop_summary(execution,confirmed_attempts,last_confirmed_at_ms) VALUES(?1,1,?2) ON CONFLICT(execution) DO UPDATE SET confirmed_attempts=confirmed_attempts+1,last_confirmed_at_ms=max(last_confirmed_at_ms,excluded.last_confirmed_at_ms)",params![execution,confirmed])?;
        tx.execute(
            "DELETE FROM session_stop_attempts WHERE attempt=?1",
            [attempt.as_str()],
        )?;
        tx.commit()?;
        Ok(())
    }
}
