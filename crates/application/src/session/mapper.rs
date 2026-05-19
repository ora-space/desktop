use ora_contracts::{Session as ContractSession, SessionStatus as ContractSessionStatus};
use ora_domain::{Session as DomainSession, SessionStatus as DomainSessionStatus};

/// Maps a domain session into the app-facing contract shape.
pub(crate) fn map_session(session: DomainSession) -> ContractSession {
    ContractSession {
        id: session.id.to_string(),
        task_id: session.task_id.to_string(),
        agent_id: session.agent_id.to_string(),
        agent_session_id: session.agent_session_id,
        status: map_session_status(session.status),
    }
}

/// Translates the internal session status into the transport-facing enum.
fn map_session_status(status: DomainSessionStatus) -> ContractSessionStatus {
    match status {
        DomainSessionStatus::Running => ContractSessionStatus::Running,
        DomainSessionStatus::Stopped => ContractSessionStatus::Stopped,
    }
}
