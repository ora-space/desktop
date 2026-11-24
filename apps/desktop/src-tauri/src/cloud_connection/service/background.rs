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
        let (renamed, ended) = self.change(|state| {
            if state.epoch != epoch {
                return (None, false);
            }
            let Some((_, session)) = state.phase.configured() else {
                return (None, false);
            };
            let Session::Active { stored: current, reach } = session else {
                return (None, false);
            };
            if current.token != stored.token {
                return (None, false);
            }
            match result {
                Ok(account) => {
                    *reach = Reach::Confirmed;
                    let renamed = (current.account != account).then(|| {
                        current.account = account;
                        current.clone()
                    });
                    (renamed, false)
                }
                Err(CallError::Rejected) => {
                    *session = Session::NeedsSignIn;
                    (None, true)
                }
                Err(CallError::Unreachable(message)) => {
                    ora_info!(operation = "cloud_connection.check", outcome = "unreachable", error = %message, "Cloud server unreachable");
                    *reach = Reach::Unreachable;
                    (None, false)
                }
                Err(CallError::Request(message)) => {
                    ora_warn!(operation = "cloud_connection.check", error = %message, "unexpected Cloud answer");
                    (None, false)
                }
            }
        });
        // The entry is deleted only when this answer ended the session it was asked about: a
        // late 401 for a token that was since signed out and replaced must not delete the entry
        // the newer sign-in wrote. Under the store lock the session is checked again, so a sign-in
        // that settled in between keeps its entry. A crash before the delete leaves a dead token
        // that the next startup check rejects again.
        if ended {
            let _store = self.inner.store_lock.lock().await;
            let still_ended = self.with_state(|state| {
                state.epoch == epoch
                    && matches!(state.phase.configured(), Some((_, Session::NeedsSignIn)))
            });
            if still_ended && let Err(error) = self.delete(&server).await {
                ora_warn!(operation = "cloud_connection.check", error = %error, "failed to delete a rejected Cloud token");
            }
        }
        if let Some(stored) = renamed {
            // Rewritten only while this exact token is still the active one, so a sign-out that
            // deleted the entry in the meantime is not undone.
            let _store = self.inner.store_lock.lock().await;
            let current = self.with_state(|state| {
                state.epoch == epoch
                    && matches!(
                        state.phase.configured(),
                        Some((_, Session::Active { stored: active, .. })) if active.token == stored.token
                    )
            });
            if current && let Err(error) = self.save(&server, stored).await {
                ora_warn!(operation = "cloud_connection.check", error = %error, "failed to refresh the stored Cloud account");
            }
        }
    }

    /// The background half of a sign-in: wait for the redirect, then hand the code to a detached
    /// task that exchanges and settles it.
    ///
    /// This task is aborted when the sign-in is cancelled or replaced, but an abort only lands at
    /// an await. Once a code has arrived the server may issue a token at any moment, so the
    /// exchange and everything after it run detached: awaiting the handle here keeps the order,
    /// and dropping it on abort lets the exchange finish and settle the token either way.
    pub(super) async fn finish_login(
        &self,
        epoch: u64,
        server: CloudServer,
        listener: LoopbackListener,
        state: String,
        verifier: String,
    ) {
        let redirect_uri = listener.redirect_uri().to_owned();
        let code = match tokio::time::timeout(self.inner.timing.login_timeout, listener.wait(state))
            .await
        {
            Err(_) => Err(CloudLoginFailure::TimedOut),
            Ok(Err(LoopbackError::Denied)) => Err(CloudLoginFailure::Denied),
            Ok(Err(LoopbackError::Io(error))) => {
                ora_warn!(operation = "cloud_connection.login", error = %error, "loopback listener failed");
                Err(CloudLoginFailure::Denied)
            }
            Ok(Ok(code)) => Ok(code),
        };
        let code = match code {
            Ok(code) => code,
            Err(failure) => {
                self.end_login(epoch, failure);
                return;
            }
        };
        let service = self.clone();
        let exchange = self.inner.runtime.spawn(async move {
            service
                .exchange_and_settle(epoch, server, code, verifier, redirect_uri)
                .await
        });
        let _ = exchange.await;
    }

    /// Ends the sign-in of `epoch` without a token, if it is still the current one.
    fn end_login(&self, epoch: u64, failure: CloudLoginFailure) {
        self.change(|current| {
            if current.epoch != epoch {
                return;
            }
            if let Some((_, slot)) = current.phase.configured() {
                *slot = Session::SignedOut {
                    failure: Some(failure),
                };
            }
        });
    }

    /// Exchanges the code and settles the issued token; never aborted.
    async fn exchange_and_settle(
        &self,
        epoch: u64,
        server: CloudServer,
        code: String,
        verifier: String,
        redirect_uri: String,
    ) {
        let exchange = CodeExchange {
            code: &code,
            verifier: &verifier,
            redirect_uri: &redirect_uri,
        };
        let issued = match self.inner.ports.api.exchange(&server, exchange).await {
            Ok(issued) => issued,
            Err(CallError::Unreachable(_)) => {
                self.end_login(epoch, CloudLoginFailure::Unreachable);
                return;
            }
            Err(CallError::Rejected | CallError::Request(_)) => {
                self.end_login(epoch, CloudLoginFailure::Rejected);
                return;
            }
        };
        let stored = StoredSession {
            token: issued.token,
            account: issued.account,
        };
        self.settle(epoch, &server, stored).await;
    }

    /// Puts an issued token where it belongs: in the secure store and the state when its sign-in
    /// is still current, otherwise nowhere, revoked on the server. Holding the store lock means no
    /// sign-out, server change or check can touch the entry between the save and the decision.
    async fn settle(&self, epoch: u64, server: &CloudServer, stored: StoredSession) {
        let store = self.inner.store_lock.lock().await;
        if self.with_state(|state| state.epoch) != epoch {
            drop(store);
            self.revoke_issued(server, &stored, "stale").await;
            return;
        }
        let saved = self.save(server, stored.clone()).await;
        let applied = self.change(|current| {
            if current.epoch != epoch {
                return false;
            }
            if let Some((_, slot)) = current.phase.configured() {
                *slot = match &saved {
                    Ok(()) => Session::Active {
                        stored: stored.clone(),
                        reach: Reach::Confirmed,
                    },
                    // Without the secure store there is nowhere safe to keep the token, so it is
                    // dropped rather than held in memory or written in plain text.
                    Err(error) => Session::StorageUnavailable {
                        reason: error.0.clone(),
                    },
                };
            }
            true
        });
        match (applied, saved) {
            (true, Ok(())) => {
                drop(store);
                self.inner.ports.observer.signed_in();
            }
            (true, Err(_)) => {
                drop(store);
                self.revoke_issued(server, &stored, "store_refused").await;
            }
            (false, saved) => {
                // The sign-in was cancelled or replaced while the token was being written. Nobody
                // else writes this entry while the store lock is held, so it is still ours.
                if saved.is_ok()
                    && let Err(error) = self.delete(server).await
                {
                    ora_warn!(operation = "cloud_connection.login", error = %error, "failed to delete the token of an abandoned sign-in");
                }
                drop(store);
                self.revoke_issued(server, &stored, "abandoned").await;
            }
        }
    }

    /// Asks the server, best effort, to revoke a token this host will not keep.
    async fn revoke_issued(&self, server: &CloudServer, stored: &StoredSession, why: &str) {
        if let Err(error) = self.inner.ports.api.logout(server, &stored.token).await {
            ora_info!(operation = "cloud_connection.login", outcome = "revoke_unconfirmed", reason = why, error = ?error, "could not revoke a token this Desktop is not keeping");
        }
    }

    /// Deletes a token locally, then asks its server to revoke it with the copy still in memory,
    /// whether or not the delete worked. Returns whether the server confirmed, and the delete's
    /// outcome.
    pub(super) async fn forget(
        &self,
        server: CloudServer,
        stored: StoredSession,
    ) -> (CloudRevocation, Result<(), StoreError>) {
        let deleted = {
            let _store = self.inner.store_lock.lock().await;
            self.delete(&server).await
        };
        if let Err(error) = &deleted {
            ora_warn!(operation = "cloud_connection.logout", error = %error, "failed to delete the Cloud token");
        }
        let revocation = match self.inner.ports.api.logout(&server, &stored.token).await {
            // 401 means the server no longer knows the token, which is what revoking wants.
            Ok(()) | Err(CallError::Rejected) => CloudRevocation::Confirmed,
            Err(CallError::Unreachable(message) | CallError::Request(message)) => {
                ora_info!(operation = "cloud_connection.logout", outcome = "unconfirmed", error = %message, "Cloud revocation not confirmed");
                CloudRevocation::Unconfirmed
            }
        };
        (revocation, deleted)
    }

    /// Reads the secure store off the async workers. Callers that act on what they read hold the
    /// store lock, as do all writers below.
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
