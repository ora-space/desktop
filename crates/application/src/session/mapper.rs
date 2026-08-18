use ora_contracts::{
    Session as ContractSession, SessionHistoryState as ContractSessionHistoryState,
    SessionStatus as ContractSessionStatus,
};
use ora_domain::{
    HistoryState as DomainHistoryState, Session as DomainSession,
    SessionStatus as DomainSessionStatus,
};

/// Maps a domain session into the app-facing contract shape.
pub(crate) fn map_session(session: DomainSession) -> ContractSession {
    ContractSession {
        id: session.id.to_string(),
        task_id: session.task_id.to_string(),
        title: session.title.map(|title| title.as_str().to_owned()),
        agent_ref: session.agent_ref.into(),
        status: map_session_status(session.status),
        history_state: map_history_state(session.history_state),
    }
}

/// Translates whether the session's history can still be extended.
fn map_history_state(history_state: DomainHistoryState) -> ContractSessionHistoryState {
    match history_state {
        DomainHistoryState::Writable => ContractSessionHistoryState::Writable,
        DomainHistoryState::Degraded { reason } => ContractSessionHistoryState::Degraded { reason },
    }
}

/// Translates the internal session status into the transport-facing enum.
fn map_session_status(status: DomainSessionStatus) -> ContractSessionStatus {
    match status {
        DomainSessionStatus::Running => ContractSessionStatus::Running,
        DomainSessionStatus::Stopped => ContractSessionStatus::Stopped,
    }
}
