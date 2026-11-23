//! The single owner of the Cloud connection: server, sign-in transaction, token and state.
//!
//! Every change happens under one lock and bumps an epoch when it invalidates work in flight (a
//! new server, a new or cancelled sign-in, a sign-out). Background work records the epoch it
//! started under and applies its result only if nothing has invalidated it since, so a late answer
//! for an old server or an abandoned sign-in can never overwrite the current state.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ora_backend::{BackendError, ErrorClassification};
use ora_contracts::{
    CloudConnection, CloudRevocation, EmptyErrorParams, LogoutCloudResponse, PublicError,
};
use ora_logging::{ora_info, ora_warn};
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::time::Instant;

mod background;
mod state;

use super::api::CloudApi;
use super::loopback::LoopbackListener;
use super::pkce::{challenge, random_token};
use super::ports::{BrowserOpener, ConnectionObserver, ServerSetting};
use super::secure_store::{SecureStore, StoreError};
use super::server::CloudServer;
use state::{Phase, Reach, Session, State};

/// How long the host waits for the browser approval, and how often a focus may re-check.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timing {
    pub(crate) login_timeout: Duration,
    pub(crate) focus_check_interval: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            login_timeout: Duration::from_secs(5 * 60),
            focus_check_interval: Duration::from_secs(60),
        }
    }
}

/// The collaborators, each behind its own small trait so tests replace exactly one effect.
pub(crate) struct Ports<S, A, B, P, O> {
    pub(crate) store: S,
    pub(crate) api: A,
    pub(crate) browser: B,
    pub(crate) setting: P,
    pub(crate) observer: O,
}

/// Owns the Cloud connection for the whole application.
pub(crate) struct CloudConnectionService<S, A, B, P, O> {
    inner: Arc<Inner<S, A, B, P, O>>,
}

impl<S, A, B, P, O> Clone for CloudConnectionService<S, A, B, P, O> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct Inner<S, A, B, P, O> {
    ports: Ports<S, A, B, P, O>,
    timing: Timing,
    runtime: Handle,
    state: Mutex<State>,
    ready: watch::Sender<bool>,
}

/// An invalid-request failure shown to the main webview.
fn invalid(context: &'static str) -> BackendError {
    BackendError::new(
        ErrorClassification::InvalidRequest,
        PublicError::InvalidRequest(EmptyErrorParams {}),
        context,
    )
}

impl<S, A, B, P, O> CloudConnectionService<S, A, B, P, O>
where
    S: SecureStore,
    A: CloudApi,
    B: BrowserOpener,
    P: ServerSetting,
    O: ConnectionObserver,
{
    /// Creates the service and restores the saved connection in the background, so reading the
    /// keyring (which may wait for an unlock prompt) never delays local startup.
    pub(crate) fn start(ports: Ports<S, A, B, P, O>, timing: Timing, runtime: Handle) -> Self {
        let (ready, _) = watch::channel(false);
        let service = Self {
            inner: Arc::new(Inner {
                ports,
                timing,
                runtime,
                state: Mutex::new(State {
                    phase: Phase::Starting,
                    epoch: 0,
                    last_check: None,
                    last_emitted: None,
                }),
                ready,
            }),
        };
        let restoring = service.clone();
        service
            .inner
            .runtime
            .spawn(async move { restoring.restore().await });
        service
    }

    /// The current connection, once startup has restored it.
    pub(crate) async fn connection(&self) -> CloudConnection {
        self.wait_ready().await;
        self.snapshot()
    }

    /// Saves a new server (or forgets it). The previous server's sign-in is cancelled and its
    /// token deleted here and revoked on that server in the background.
    pub(crate) async fn set_server(
        &self,
        input: Option<String>,
    ) -> Result<CloudConnection, BackendError> {
        let target = match input.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(text) => Some(
                CloudServer::parse(text).map_err(|_| invalid("invalid Cloud server address"))?,
            ),
        };
        self.wait_ready().await;
        let current = self.with_state(|state| match &state.phase {
            Phase::Configured { server, .. } => Some(server.clone()),
            Phase::Starting | Phase::NotConfigured => None,
        });
        if current == target {
            return Ok(self.snapshot());
        }
        self.inner
            .ports
            .setting
            .store(target.as_ref().map(|server| server.origin().to_owned()))
            .await
            .map_err(|message| {
                BackendError::internal("failed to save the Cloud server", StoreError(message))
            })?;
        let previous = self.change(|state| {
            state.epoch += 1;
            let next = match target {
                Some(server) => Phase::Configured {
                    server,
                    session: Session::SignedOut { failure: None },
                },
                None => Phase::NotConfigured,
            };
            match std::mem::replace(&mut state.phase, next) {
                Phase::Configured { server, session } => {
                    session.abort();
                    match session {
                        Session::Active { stored, .. } => Some((server, stored)),
                        Session::SignedOut { .. }
                        | Session::SigningIn { .. }
                        | Session::NeedsSignIn
                        | Session::StorageUnavailable { .. } => None,
                    }
                }
                Phase::Starting | Phase::NotConfigured => None,
            }
        });
        if let Some((server, stored)) = previous {
            self.forget(server, stored).await;
        }
        Ok(self.snapshot())
    }

    /// Opens the browser on the server's approval page and waits for the redirect in the
    /// background. A sign-in already in progress is replaced.
    pub(crate) async fn start_login(&self) -> Result<CloudConnection, BackendError> {
        self.wait_ready().await;
        let server = self.with_state(|state| match state.phase.configured() {
            Some((server, session)) => match session {
                Session::Active { .. } => Err(invalid("already signed in to the Cloud server")),
                Session::SignedOut { .. }
                | Session::SigningIn { .. }
                | Session::NeedsSignIn
                | Session::StorageUnavailable { .. } => Ok(server.clone()),
            },
            None => Err(invalid("no Cloud server is configured")),
        })?;
        let listener = LoopbackListener::bind()
            .await
            .map_err(|error| BackendError::internal("failed to listen for the sign-in", error))?;
        let state = random_token();
        let verifier = random_token();
        let mut authorize = url::Url::parse(&server.endpoint("/desktop/authorize"))
            .map_err(|error| BackendError::internal("invalid Cloud authorize address", error))?;
        authorize
            .query_pairs_mut()
            .append_pair("redirect_uri", listener.redirect_uri())
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge(&verifier))
            .append_pair("code_challenge_method", "S256");
        let inner = Arc::clone(&self.inner);
        let url = authorize.to_string();
        tokio::task::spawn_blocking(move || inner.ports.browser.open(&url))
            .await
            .map_err(|error| BackendError::internal("browser launch did not complete", error))?
            .map_err(|message| {
                BackendError::new(
                    ErrorClassification::HostUnavailable,
                    PublicError::InternalError(EmptyErrorParams {}),
                    format!("failed to open the system browser: {message}"),
                )
            })?;
        let service = self.clone();
        self.change(|current| {
            let Some((configured, session)) = current.phase.configured() else {
                return;
            };
            if *configured != server {
                return;
            }
            current.epoch += 1;
            session.abort();
            let epoch = current.epoch;
            let task = self
                .inner
                .runtime
                .spawn(async move {
                    service
                        .finish_login(epoch, server, listener, state, verifier)
                        .await
                })
                .abort_handle();
            *session = Session::SigningIn { task };
        });
        ora_info!(
            operation = "cloud_connection.login",
            outcome = "browser_opened",
            "Cloud sign-in started"
        );
        Ok(self.snapshot())
    }

    /// Abandons a sign-in in progress; anything else is left as it is.
    pub(crate) async fn cancel_login(&self) -> CloudConnection {
        self.wait_ready().await;
        self.change(|state| {
            if let Some((_, session)) = state.phase.configured()
                && let Session::SigningIn { task } = session
            {
                task.abort();
                *session = Session::SignedOut { failure: None };
                state.epoch += 1;
            }
        });
        self.snapshot()
    }

    /// Signs out: deletes the token from the secure store, forgets it in memory and asks the
    /// server to revoke it. The local state is cleared even when the server is unreachable; when
    /// the secure store refuses the delete, the session reports the store instead of "signed out".
    pub(crate) async fn logout(&self) -> LogoutCloudResponse {
        self.wait_ready().await;
        let held = self.change(|state| {
            let (server, session) = state.phase.configured()?;
            let server = server.clone();
            session.abort();
            let previous = std::mem::replace(session, Session::SignedOut { failure: None });
            state.epoch += 1;
            match previous {
                Session::Active { stored, .. } => Some((state.epoch, server, stored)),
                Session::SignedOut { .. }
                | Session::SigningIn { .. }
                | Session::NeedsSignIn
                | Session::StorageUnavailable { .. } => None,
            }
        });
        let revocation = match held {
            Some((epoch, server, stored)) => {
                let (revocation, deleted) = self.forget(server, stored).await;
                if let Err(error) = deleted {
                    self.change(|state| {
                        if state.epoch != epoch {
                            return;
                        }
                        if let Some((_, session)) = state.phase.configured()
                            && matches!(session, Session::SignedOut { failure: None })
                        {
                            *session = Session::StorageUnavailable { reason: error.0 };
                        }
                    });
                }
                revocation
            }
            None => CloudRevocation::Confirmed,
        };
        LogoutCloudResponse {
            connection: self.snapshot(),
            revocation,
        }
    }

    /// Re-checks the token when the main window regains focus, at most once per interval.
    pub(crate) fn window_focused(&self) {
        let due = self.with_state(|state| {
            let now = Instant::now();
            let due = state.last_check.is_none_or(|last| {
                now.duration_since(last) >= self.inner.timing.focus_check_interval
            });
            if due {
                state.last_check = Some(now);
            }
            due
        });
        if due {
            let service = self.clone();
            self.inner
                .runtime
                .spawn(async move { service.check().await });
        }
    }

    /// Restores the saved server and token at startup, then checks the token.
    async fn restore(&self) {
        let saved = match self.inner.ports.setting.load().await {
            Ok(saved) => saved,
            Err(message) => {
                ora_warn!(operation = "cloud_connection.restore", error = %message, "failed to read the Cloud server setting");
                None
            }
        };
        let server = saved.and_then(|origin| CloudServer::parse(&origin).ok());
        let phase = match server {
            None => Phase::NotConfigured,
            Some(server) => {
                let session = match self.load(&server).await {
                    Ok(Some(stored)) => Session::Active {
                        stored,
                        reach: Reach::Confirmed,
                    },
                    Ok(None) => Session::SignedOut { failure: None },
                    Err(error) => Session::StorageUnavailable { reason: error.0 },
                };
                Phase::Configured { server, session }
            }
        };
        let check = matches!(
            phase,
            Phase::Configured {
                session: Session::Active { .. },
                ..
            }
        );
        self.change(|state| {
            state.phase = phase;
            state.last_check = Some(Instant::now());
        });
        self.inner.ready.send_replace(true);
        if check {
            self.check().await;
        }
    }

    /// Waits until startup restored the saved connection.
    async fn wait_ready(&self) {
        let mut ready = self.inner.ready.subscribe();
        // The sender lives in `inner`, which `self` keeps alive, so the wait cannot fail.
        let _ = ready.wait_for(|ready| *ready).await;
    }

    /// The current connection without waiting for startup.
    fn snapshot(&self) -> CloudConnection {
        self.with_state(|state| state.phase.connection())
    }

    /// Reads or adjusts the state without announcing anything.
    fn with_state<T>(&self, read: impl FnOnce(&mut State) -> T) -> T {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("Cloud connection state mutex is not poisoned");
        read(&mut state)
    }

    /// Changes the state and tells the observer when the visible connection differs.
    fn change<T>(&self, update: impl FnOnce(&mut State) -> T) -> T {
        let (result, announce) = self.with_state(|state| {
            let result = update(state);
            let connection = state.phase.connection();
            let starting = matches!(state.phase, Phase::Starting);
            let announce =
                (!starting && state.last_emitted.as_ref() != Some(&connection)).then(|| {
                    state.last_emitted = Some(connection.clone());
                    connection
                });
            (result, announce)
        });
        if let Some(connection) = announce {
            self.inner.ports.observer.changed(&connection);
        }
        result
    }
}
