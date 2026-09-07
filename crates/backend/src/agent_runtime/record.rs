//! Ownership of one session's durable record: opening it, settling a failed write, and
//! recovering it.
//!
//! Separated from the runtime because these are the only three places that decide a session's
//! `HistoryState`. Stopping recording and permitting it to resume are the same judgement read in
//! opposite directions, so they belong together rather than beside the operations that happen to
//! trigger them.

use super::{
    AgentRuntimeManager, LocalHistoryClock, RecordOutcome, SessionRecorder, binding_needs_handoff,
    contract_session,
};
use crate::BackendError;
use crate::error::ErrorClassification;
use ora_application::{Clock, SessionRepository};
use ora_contracts::{
    EmptyErrorParams, PublicError, ResumeSessionHistoryRequest, ResumeSessionHistoryResponse,
};
use ora_db::SqliteSessionRepository;
use ora_domain::{HistoryState, Session, SessionId};
use ora_history::{HistoryIntegrity, read_session_history};
use ora_logging::ora_warn;

/// One session's opened recorder together with what reading its file revealed.
pub(super) struct OpenedRecorder {
    pub recorder: SessionRecorder,
    pub handoff_pending: bool,
    /// Set when the history could not be read, which degrades the session.
    ///
    /// A history Ora cannot read is one it cannot safely extend: appending
    /// without knowing the positions already used would overwrite them.
    pub failure: Option<String>,
}

impl AgentRuntimeManager {
    /// Returns a session whose history writes failed to a writable state.
    ///
    /// The gap is recorded before anything else, so what the failure cost stays
    /// visible to everyone who reads the file afterwards — including the agent
    /// that receives this conversation next.
    pub(crate) async fn resume_history(
        &self,
        request: ResumeSessionHistoryRequest,
    ) -> Result<ResumeSessionHistoryResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        let HistoryState::Degraded { reason } = session.history_state.clone() else {
            return Ok(ResumeSessionHistoryResponse {
                session: contract_session(session),
            });
        };
        // The live actor still holds a stopped recorder, so it is discarded and
        // rebuilt from the recovered row on the session's next operation.
        if let Some(handle) = self.lookup_actor(&session.id)? {
            self.stop_actor(handle).await?;
        }
        self.actors_write()?.remove(&session.id);

        let mut opened = self.open_recorder(&session)?;
        if let Some(failure) = opened.failure {
            return Err(BackendError::new(
                ErrorClassification::Internal,
                PublicError::SessionHistoryDegraded(EmptyErrorParams {}),
                format!("session history is still unreadable: {failure}"),
            ));
        }
        if let RecordOutcome::JustFailed { reason } = opened.recorder.resume(reason) {
            return Err(BackendError::new(
                ErrorClassification::Internal,
                PublicError::SessionHistoryDegraded(EmptyErrorParams {}),
                format!("session history is still unwritable: {reason}"),
            ));
        }
        let now = self.inner.clock.now_timestamp_millis();
        let session = SqliteSessionRepository::new(self.inner.pool.clone())
            .update_session_history_state(
                &SessionId::new(request.session_id.clone()),
                &HistoryState::Writable,
                now,
            )
            .map_err(|source| BackendError::internal("failed to resume session history", source))?;
        Ok(ResumeSessionHistoryResponse {
            session: contract_session(session),
        })
    }

    /// Opens one session's recorder, resuming its position counter from the file.
    pub(super) fn open_recorder(&self, session: &Session) -> Result<OpenedRecorder, BackendError> {
        let root = &self.inner.sessions_root;
        let session_id = session.id.as_ref();
        match read_session_history(root, session_id) {
            Ok(history) => {
                if let HistoryIntegrity::Damaged { unreadable_lines } = history.integrity {
                    ora_warn!(
                        session_id = %session.id,
                        unreadable_lines = unreadable_lines.get(),
                        "session history contains unreadable lines",
                    );
                }
                let recorder = SessionRecorder::open(
                    root,
                    session_id,
                    history.next_seq,
                    &session.history_state,
                    LocalHistoryClock,
                )
                .map_err(|source| {
                    BackendError::internal("failed to open session history", source)
                })?;
                Ok(OpenedRecorder {
                    recorder,
                    handoff_pending: binding_needs_handoff(&history),
                    failure: None,
                })
            }
            Err(error) => {
                // Appending without knowing which positions are already used would
                // overwrite them, so an unreadable file stops recording outright.
                ora_warn!(session_id = %session.id, error = %error, "session history is unreadable");
                let failure = error.to_string();
                let recorder = SessionRecorder::open(
                    root,
                    session_id,
                    0,
                    &HistoryState::Degraded {
                        reason: failure.clone(),
                    },
                    LocalHistoryClock,
                )
                .map_err(|source| {
                    BackendError::internal("failed to open session history", source)
                })?;
                Ok(OpenedRecorder {
                    recorder,
                    handoff_pending: false,
                    failure: Some(failure),
                })
            }
        }
    }

    /// Persists the degraded state when a recording attempt just broke the history.
    pub(super) fn settle_record(&self, session: Session, outcome: RecordOutcome) -> Session {
        let RecordOutcome::JustFailed { reason } = outcome else {
            return session;
        };
        let now = self.inner.clock.now_timestamp_millis();
        let degraded = session.with_history_state(HistoryState::Degraded { reason }, now);
        match SqliteSessionRepository::new(self.inner.pool.clone()).update_session_history_state(
            &degraded.id,
            &degraded.history_state,
            now,
        ) {
            Ok(stored) => stored,
            Err(error) => {
                ora_warn!(error = %error, "failed to persist degraded session history state");
                degraded
            }
        }
    }
}
