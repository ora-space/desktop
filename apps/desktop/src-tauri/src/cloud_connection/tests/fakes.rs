//! In-memory stand-ins for each port, sharing their state with the test that inspects them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ora_contracts::{CloudAccount, CloudConnection};
use tokio::runtime::Handle;

use super::super::api::{CallError, CloudApi, CodeExchange, IssuedToken};
use super::super::ports::{BrowserOpener, ConnectionObserver, ServerSetting};
use super::super::secret::SessionToken;
use super::super::secure_store::{SecureStore, StoreError, StoredSession};
use super::super::server::CloudServer;

/// A secure store that can be told to fail.
#[derive(Clone, Default)]
pub(super) struct MemoryStore {
    pub(super) entries: Arc<Mutex<HashMap<String, StoredSession>>>,
    pub(super) failure: Arc<Mutex<Option<String>>>,
}

impl MemoryStore {
    /// Fails like a locked or missing Secret Service when a failure is set.
    fn check(&self) -> Result<(), StoreError> {
        match self.failure.lock().expect("failure lock").clone() {
            Some(reason) => Err(StoreError(reason)),
            None => Ok(()),
        }
    }

    /// The token stored for an origin, as a plain string for comparisons.
    pub(super) fn token(&self, origin: &str) -> Option<String> {
        self.entries
            .lock()
            .expect("entries lock")
            .get(origin)
            .map(|stored| stored.token.expose().to_owned())
    }
}

impl SecureStore for MemoryStore {
    /// Reads an entry unless failing.
    fn load(&self, server: &CloudServer) -> Result<Option<StoredSession>, StoreError> {
        self.check()?;
        Ok(self
            .entries
            .lock()
            .expect("entries lock")
            .get(server.origin())
            .cloned())
    }

    /// Writes an entry unless failing.
    fn save(&self, server: &CloudServer, session: &StoredSession) -> Result<(), StoreError> {
        self.check()?;
        self.entries
            .lock()
            .expect("entries lock")
            .insert(server.origin().to_owned(), session.clone());
        Ok(())
    }

    /// Deletes an entry unless failing.
    fn delete(&self, server: &CloudServer) -> Result<(), StoreError> {
        self.check()?;
        self.entries
            .lock()
            .expect("entries lock")
            .remove(server.origin());
        Ok(())
    }
}

/// Canned Cloud answers plus a log of every call as `route origin token`.
#[derive(Clone)]
pub(super) struct ScriptedApi {
    pub(super) exchange: Arc<Mutex<Result<String, CallError>>>,
    pub(super) me: Arc<Mutex<Result<CloudAccount, CallError>>>,
    pub(super) logout: Arc<Mutex<Result<(), CallError>>>,
    /// When set, the next `me` call waits for this before answering, holding a check in flight.
    pub(super) me_gate: Arc<Mutex<Option<Arc<tokio::sync::Notify>>>>,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
    pub(super) exchanges: Arc<Mutex<Vec<(String, String, String)>>>,
}

impl ScriptedApi {
    /// Answers every call successfully for `account`.
    pub(super) fn new(account: CloudAccount) -> Self {
        Self {
            exchange: Arc::new(Mutex::new(Ok("ora_dt_issued".to_owned()))),
            me: Arc::new(Mutex::new(Ok(account))),
            logout: Arc::new(Mutex::new(Ok(()))),
            me_gate: Arc::default(),
            calls: Arc::default(),
            exchanges: Arc::default(),
        }
    }

    /// Records one call.
    fn record(&self, route: &str, server: &CloudServer, token: &str) {
        self.calls
            .lock()
            .expect("calls lock")
            .push(format!("{route} {} {token}", server.origin()));
    }
}

impl CloudApi for ScriptedApi {
    /// Returns the canned token for whatever account `me` would report.
    async fn exchange(
        &self,
        server: &CloudServer,
        exchange: CodeExchange<'_>,
    ) -> Result<IssuedToken, CallError> {
        self.record("exchange", server, "-");
        self.exchanges.lock().expect("exchanges lock").push((
            exchange.code.to_owned(),
            exchange.verifier.to_owned(),
            exchange.redirect_uri.to_owned(),
        ));
        let token = self.exchange.lock().expect("exchange lock").clone()?;
        let account = match self.me.lock().expect("me lock").clone() {
            Ok(account) => account,
            Err(_) => CloudAccount {
                id: "u".to_owned(),
                display_name: "someone".to_owned(),
            },
        };
        Ok(IssuedToken {
            token: SessionToken::new(token),
            account,
        })
    }

    /// Returns the canned account check.
    async fn me(
        &self,
        server: &CloudServer,
        token: &SessionToken,
    ) -> Result<CloudAccount, CallError> {
        self.record("me", server, token.expose());
        let answer = self.me.lock().expect("me lock").clone();
        let gate = self.me_gate.lock().expect("gate lock").take();
        if let Some(gate) = gate {
            gate.notified().await;
        }
        answer
    }

    /// Returns the canned revocation.
    async fn logout(&self, server: &CloudServer, token: &SessionToken) -> Result<(), CallError> {
        self.record("logout", server, token.expose());
        self.logout.lock().expect("logout lock").clone()
    }
}

/// What the stand-in browser does with the approval page.
#[derive(Clone, Debug)]
pub(super) enum BrowserAction {
    /// Only remember the address.
    Record,
    /// Approve: request the redirect URI with this code and the page's state.
    Approve(&'static str),
    /// Cancel on the page: request the redirect URI with an error.
    Deny,
    /// Fail to launch.
    Fail,
}

/// A browser that records the approval address and optionally answers it like the Cloud page.
#[derive(Clone)]
pub(super) struct FakeBrowser {
    pub(super) action: Arc<Mutex<BrowserAction>>,
    pub(super) opened: Arc<Mutex<Vec<String>>>,
    runtime: Handle,
}

impl FakeBrowser {
    pub(super) fn new(action: BrowserAction) -> Self {
        Self {
            action: Arc::new(Mutex::new(action)),
            opened: Arc::default(),
            runtime: Handle::current(),
        }
    }
}

impl BrowserOpener for FakeBrowser {
    /// Plays the person in the browser for the configured action.
    fn open(&self, url: &str) -> Result<(), String> {
        self.opened
            .lock()
            .expect("opened lock")
            .push(url.to_owned());
        let query = match self.action.lock().expect("action lock").clone() {
            BrowserAction::Record => return Ok(()),
            BrowserAction::Fail => return Err("no browser".to_owned()),
            BrowserAction::Approve(code) => format!("code={code}"),
            BrowserAction::Deny => "error=access_denied".to_owned(),
        };
        let (redirect, state) = redirect_and_state(url);
        self.runtime.spawn(async move {
            super::send_redirect(&redirect, &format!("{query}&state={state}")).await
        });
        Ok(())
    }
}

/// Reads the redirect URI and state the host put on the approval address.
pub(super) fn redirect_and_state(url: &str) -> (String, String) {
    let url = url::Url::parse(url).expect("approval url");
    let value = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .expect("approval parameter")
    };
    (value("redirect_uri"), value("state"))
}

/// The saved server origin.
#[derive(Clone, Default)]
pub(super) struct MemorySetting {
    pub(super) origin: Arc<Mutex<Option<String>>>,
}

impl ServerSetting for MemorySetting {
    /// Reads the saved origin.
    async fn load(&self) -> Result<Option<String>, String> {
        Ok(self.origin.lock().expect("origin lock").clone())
    }

    /// Replaces the saved origin.
    async fn store(&self, origin: Option<String>) -> Result<(), String> {
        *self.origin.lock().expect("origin lock") = origin;
        Ok(())
    }
}

/// Collects announcements.
#[derive(Clone, Default)]
pub(super) struct RecordingObserver {
    pub(super) changes: Arc<Mutex<Vec<CloudConnection>>>,
    pub(super) sign_ins: Arc<Mutex<u32>>,
}

impl ConnectionObserver for RecordingObserver {
    /// Records the connection.
    fn changed(&self, connection: &CloudConnection) {
        self.changes
            .lock()
            .expect("changes lock")
            .push(connection.clone());
    }

    /// Counts completed sign-ins.
    fn signed_in(&self) {
        *self.sign_ins.lock().expect("sign-ins lock") += 1;
    }
}
