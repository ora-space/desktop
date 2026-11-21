//! Work the connection runs in the background: checking the token, finishing a sign-in, and the
//! secure-store calls that must stay off the async workers.

use std::sync::Arc;

use ora_contracts::{CloudLoginFailure, CloudRevocation};
use ora_logging::{ora_info, ora_warn};

use super::CloudConnectionService;
use super::state::{Reach, Session};
use crate::cloud_connection::api::{CallError, CloudApi, CodeExchange};
use crate::cloud_connection::loopback::{LoopbackError, LoopbackListener};
use crate::cloud_connection::ports::{BrowserOpener, ConnectionObserver, ServerSetting};
use crate::cloud_connection::secure_store::{SecureStore, StoreError, StoredSession};
use crate::cloud_connection::server::CloudServer;

impl<S, A, B, P, O> CloudConnectionService<S, A, B, P, O>
where
    S: SecureStore,
    A: CloudApi,
    B: BrowserOpener,
    P: ServerSetting,
    O: ConnectionObserver,
{
    /// Asks the server who the token belongs to and applies the session rule to the answer.
    pub(super) async fn check(&self) {
        let Some((epoch, server, stored)) = self.with_state(|state| {
            let epoch = state.epoch;
            match state.phase.configured() {
                Some((server, Session::Active { stored, .. })) => {
                    Some((epoch, server.clone(), stored.clone()))
                }
                Some(_) | None => None,
            }
        }) else {
            return;
        };
        let result = self.inner.ports.api.me(&server, &stored.token).await;
        if matches!(result, Err(CallError::Rejected)) {
            // Deleting first means a crash right after leaves no token that is known to be dead.
            if let Err(error) = self.delete(&server).await {
                ora_warn!(operation = "cloud_connection.check", error = %error, "failed to delete a rejected Cloud token");
            }
        }
        let renamed = self.change(|state| {
            if state.epoch != epoch {
                return None;
            }
            let (_, session) = state.phase.configured()?;
            let Session::Active { stored: current, reach } = session else {
                return None;
            };
            if current.token != stored.token {
                return None;
            }
            match result {
                Ok(account) => {
                    *reach = Reach::Confirmed;
                    (current.account != account).then(|| {
                        current.account = account;
                        current.clone()
                    })
                }
                Err(CallError::Rejected) => {
                    *session = Session::NeedsSignIn;
                    None
                }
                Err(CallError::Unreachable(message)) => {
                    ora_info!(operation = "cloud_connection.check", outcome = "unreachable", error = %message, "Cloud server unreachable");
                    *reach = Reach::Unreachable;
                    None
                }
                Err(CallError::Request(message)) => {
                    ora_warn!(operation = "cloud_connection.check", error = %message, "unexpected Cloud answer");
                    None
                }
            }
        });
        if let Some(stored) = renamed
            && let Err(error) = self.save(&server, stored).await
        {
            ora_warn!(operation = "cloud_connection.check", error = %error, "failed to refresh the stored Cloud account");
        }
    }

    /// The background half of a sign-in: wait for the redirect, exchange the code, store the token.
    pub(super) async fn finish_login(
        &self,
        epoch: u64,
        server: CloudServer,
        listener: LoopbackListener,
        state: String,
        verifier: String,
    ) {
        let redirect_uri = listener.redirect_uri().to_owned();
        let outcome = match tokio::time::timeout(
            self.inner.timing.login_timeout,
            listener.wait(state),
        )
        .await
        {
            Err(_) => Err(CloudLoginFailure::TimedOut),
            Ok(Err(LoopbackError::Denied)) => Err(CloudLoginFailure::Denied),
            Ok(Err(LoopbackError::Io(error))) => {
                ora_warn!(operation = "cloud_connection.login", error = %error, "loopback listener failed");
                Err(CloudLoginFailure::Denied)
            }
            Ok(Ok(code)) => {
                let exchange = CodeExchange {
                    code: &code,
                    verifier: &verifier,
                    redirect_uri: &redirect_uri,
                };
                match self.inner.ports.api.exchange(&server, exchange).await {
                    Ok(issued) => Ok(issued),
                    Err(CallError::Unreachable(_)) => Err(CloudLoginFailure::Unreachable),
                    Err(CallError::Rejected | CallError::Request(_)) => {
                        Err(CloudLoginFailure::Rejected)
                    }
                }
            }
        };
        let session = match outcome {
            Err(failure) => Session::SignedOut {
                failure: Some(failure),
            },
            Ok(issued) => {
                let stored = StoredSession {
                    token: issued.token,
                    account: issued.account,
                };
                match self.save(&server, stored.clone()).await {
                    // Without the secure store there is nowhere safe to keep the token, so it is
                    // dropped rather than held in memory or written in plain text.
                    Err(error) => Session::StorageUnavailable { reason: error.0 },
                    Ok(()) => Session::Active {
                        stored,
                        reach: Reach::Confirmed,
                    },
                }
            }
        };
        let signed_in = matches!(session, Session::Active { .. });
        let applied = self.change(|current| {
            if current.epoch != epoch {
                return false;
            }
            if let Some((_, slot)) = current.phase.configured() {
                *slot = session;
            }
            true
        });
        if applied && signed_in {
            self.inner.ports.observer.signed_in();
        }
    }

    /// Revokes a token on its server and deletes it locally; reports whether the server confirmed.
    pub(super) async fn forget(
        &self,
        server: CloudServer,
        stored: StoredSession,
    ) -> CloudRevocation {
        if let Err(error) = self.delete(&server).await {
            ora_warn!(operation = "cloud_connection.logout", error = %error, "failed to delete the Cloud token");
        }
        match self.inner.ports.api.logout(&server, &stored.token).await {
            // 401 means the server no longer knows the token, which is what revoking wants.
            Ok(()) | Err(CallError::Rejected) => CloudRevocation::Confirmed,
            Err(CallError::Unreachable(message) | CallError::Request(message)) => {
                ora_info!(operation = "cloud_connection.logout", outcome = "unconfirmed", error = %message, "Cloud revocation not confirmed");
                CloudRevocation::Unconfirmed
            }
        }
    }

    /// Reads the secure store off the async workers.
    pub(super) async fn load(
        &self,
        server: &CloudServer,
    ) -> Result<Option<StoredSession>, StoreError> {
        let inner = Arc::clone(&self.inner);
        let server = server.clone();
        tokio::task::spawn_blocking(move || inner.ports.store.load(&server))
            .await
            .unwrap_or_else(|error| Err(StoreError(error.to_string())))
    }

    /// Writes the secure store off the async workers.
    pub(super) async fn save(
        &self,
        server: &CloudServer,
        stored: StoredSession,
    ) -> Result<(), StoreError> {
        let inner = Arc::clone(&self.inner);
        let server = server.clone();
        tokio::task::spawn_blocking(move || inner.ports.store.save(&server, &stored))
            .await
            .unwrap_or_else(|error| Err(StoreError(error.to_string())))
    }

    /// Deletes from the secure store off the async workers.
    pub(super) async fn delete(&self, server: &CloudServer) -> Result<(), StoreError> {
        let inner = Arc::clone(&self.inner);
        let server = server.clone();
        tokio::task::spawn_blocking(move || inner.ports.store.delete(&server))
            .await
            .unwrap_or_else(|error| Err(StoreError(error.to_string())))
    }
}
