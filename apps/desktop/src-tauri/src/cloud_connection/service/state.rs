//! The connection's internal state and its projection for the main webview.

use ora_contracts::{CloudConnection, CloudLoginFailure, CloudSession};
use tokio::task::AbortHandle;
use tokio::time::Instant;

use crate::cloud_connection::secure_store::StoredSession;
use crate::cloud_connection::server::CloudServer;

pub(super) struct State {
    pub(super) phase: Phase,
    pub(super) epoch: u64,
    pub(super) last_check: Option<Instant>,
    pub(super) last_emitted: Option<CloudConnection>,
}

/// Internal state; the token exists only inside `Active`.
pub(super) enum Phase {
    Starting,
    NotConfigured,
    Configured {
        server: CloudServer,
        session: Session,
    },
}

pub(super) enum Session {
    SignedOut { failure: Option<CloudLoginFailure> },
    SigningIn { task: AbortHandle },
    Active { stored: StoredSession, reach: Reach },
    NeedsSignIn,
    StorageUnavailable { reason: String },
}

#[derive(Clone, Copy)]
pub(super) enum Reach {
    Confirmed,
    Unreachable,
}

impl Phase {
    /// Projects the state into what the main webview may see; `Starting` looks unconfigured but
    /// is never returned by a command, which waits for startup first.
    pub(super) fn connection(&self) -> CloudConnection {
        let Phase::Configured { server, session } = self else {
            return CloudConnection::NotConfigured;
        };
        let session = match session {
            Session::SignedOut { failure } => CloudSession::SignedOut { failure: *failure },
            Session::SigningIn { .. } => CloudSession::SigningIn,
            Session::Active { stored, reach } => match reach {
                Reach::Confirmed => CloudSession::SignedIn {
                    account: stored.account.clone(),
                },
                Reach::Unreachable => CloudSession::Unreachable {
                    account: stored.account.clone(),
                },
            },
            Session::NeedsSignIn => CloudSession::NeedsSignIn,
            Session::StorageUnavailable { reason } => CloudSession::StorageUnavailable {
                reason: reason.clone(),
            },
        };
        CloudConnection::Configured {
            server: server.origin().to_owned(),
            session,
        }
    }

    /// The server and its current session, when one is configured.
    pub(super) fn configured(&mut self) -> Option<(&CloudServer, &mut Session)> {
        match self {
            Phase::Configured { server, session } => Some((server, session)),
            Phase::Starting | Phase::NotConfigured => None,
        }
    }
}

impl Session {
    /// Stops a sign-in in progress; dropping its task closes the loopback listener.
    pub(super) fn abort(&self) {
        if let Session::SigningIn { task } = self {
            task.abort();
        }
    }
}
