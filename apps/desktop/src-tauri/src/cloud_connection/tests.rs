//! Behaviour of the Cloud connection through its public use cases, with each port replaced.

mod end_to_end;
mod fakes;
mod stub_cloud;

use std::time::Duration;

use fakes::{
    BrowserAction, FakeBrowser, MemorySetting, MemoryStore, RecordingObserver, ScriptedApi,
};
use ora_contracts::{
    CloudAccount, CloudConnection, CloudLoginFailure, CloudRevocation, CloudSession,
};
use pretty_assertions::assert_eq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::Handle;

use super::api::CallError;
use super::pkce::challenge;
use super::secret::SessionToken;
use super::secure_store::StoredSession;
use super::service::{CloudConnectionService, Ports, Timing};

const SERVER: &str = "https://cloud.example";

type TestService =
    CloudConnectionService<MemoryStore, ScriptedApi, FakeBrowser, MemorySetting, RecordingObserver>;

/// One service and handles on every fake behind it.
struct Harness {
    service: TestService,
    store: MemoryStore,
    api: ScriptedApi,
    browser: FakeBrowser,
    setting: MemorySetting,
    observer: RecordingObserver,
}

fn alice() -> CloudAccount {
    CloudAccount {
        id: "u-alice".to_owned(),
        display_name: "Alice".to_owned(),
    }
}

/// Starts a service with `server` saved and, optionally, a stored token for it.
fn harness(
    server: Option<&str>,
    token: Option<&str>,
    action: BrowserAction,
    timing: Timing,
) -> Harness {
    let store = MemoryStore::default();
    if let (Some(server), Some(token)) = (server, token) {
        store.entries.lock().expect("entries").insert(
            server.to_owned(),
            StoredSession {
                token: SessionToken::new(token.to_owned()),
                account: alice(),
            },
        );
    }
    let setting = MemorySetting::default();
    *setting.origin.lock().expect("origin") = server.map(str::to_owned);
    let api = ScriptedApi::new(alice());
    let browser = FakeBrowser::new(action);
    let observer = RecordingObserver::default();
    let service = CloudConnectionService::start(
        Ports {
            store: store.clone(),
            api: api.clone(),
            browser: browser.clone(),
            setting: setting.clone(),
            observer: observer.clone(),
        },
        timing,
        Handle::current(),
    );
    Harness {
        service,
        store,
        api,
        browser,
        setting,
        observer,
    }
}

fn configured(session: CloudSession) -> CloudConnection {
    CloudConnection::Configured {
        server: SERVER.to_owned(),
        session,
    }
}

/// Polls until the connection matches, failing after a few seconds.
async fn eventually(service: &TestService, expected: CloudConnection) {
    eventually_for(service, expected).await;
}

/// `eventually` for a service with any ports.
async fn eventually_for<S, A, B, P, O>(
    service: &CloudConnectionService<S, A, B, P, O>,
    expected: CloudConnection,
) where
    S: super::secure_store::SecureStore,
    A: super::api::CloudApi,
    B: super::ports::BrowserOpener,
    P: super::ports::ServerSetting,
    O: super::ports::ConnectionObserver,
{
    for _ in 0..500 {
        if service.connection().await == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(service.connection().await, expected);
}

/// Requests a redirect URI the way a browser would; `Err` when nothing listens any more.
async fn send_redirect(redirect_uri: &str, query: &str) -> std::io::Result<String> {
    let url = url::Url::parse(redirect_uri).expect("redirect uri");
    let address = format!("127.0.0.1:{}", url.port().expect("port"));
    let mut stream = TcpStream::connect(&address).await?;
    stream
        .write_all(
            format!(
                "GET {}?{query} HTTP/1.1\r\nHost: {address}\r\n\r\n",
                url.path()
            )
            .as_bytes(),
        )
        .await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    Ok(response.lines().next().unwrap_or_default().to_owned())
}

/// The approval address, the redirect it names and the challenge it carries.
fn opened(harness: &Harness) -> (String, String, String) {
    let url = harness
        .browser
        .opened
        .lock()
        .expect("opened")
        .last()
        .cloned()
        .expect("browser opened");
    let (redirect, state) = fakes::redirect_and_state(&url);
    let parsed = url::Url::parse(&url).expect("approval url");
    let challenge = parsed
        .query_pairs()
        .find(|(key, _)| key == "code_challenge")
        .map(|(_, value)| value.into_owned())
        .expect("challenge");
    assert!(
        url.starts_with("https://cloud.example/desktop/authorize?"),
        "{url}"
    );
    assert_eq!(state.len(), 43);
    (url, redirect, challenge)
}

/// A complete sign-in: approval in the browser, exchange with the matching verifier, token stored.
#[tokio::test(flavor = "multi_thread")]
async fn signs_in_through_the_browser_and_stores_the_token() {
    let h = harness(
        Some(SERVER),
        None,
        BrowserAction::Approve("code-1"),
        Timing::default(),
    );
    assert_eq!(
        h.service.connection().await,
        configured(CloudSession::SignedOut { failure: None })
    );
    h.service.start_login().await.expect("start");
    eventually(
        &h.service,
        configured(CloudSession::SignedIn { account: alice() }),
    )
    .await;

    let (_, redirect, sent_challenge) = opened(&h);
    let exchanges = h.api.exchanges.lock().expect("exchanges").clone();
    assert_eq!(exchanges.len(), 1);
    let (code, verifier, exchanged_redirect) = exchanges[0].clone();
    assert_eq!(
        (code, exchanged_redirect, challenge(&verifier)),
        ("code-1".to_owned(), redirect, sent_challenge)
    );
    assert_eq!(h.store.token(SERVER), Some("ora_dt_issued".to_owned()));
    assert_eq!(*h.observer.sign_ins.lock().expect("sign-ins"), 1);
    assert!(
        h.observer
            .changes
            .lock()
            .expect("changes")
            .contains(&configured(CloudSession::SigningIn))
    );
}

/// Cancelling on the browser page ends the sign-in without a token and says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_denied_approval_signs_nothing_in() {
    let h = harness(Some(SERVER), None, BrowserAction::Deny, Timing::default());
    h.service.start_login().await.expect("start");
    eventually(
        &h.service,
        configured(CloudSession::SignedOut {
            failure: Some(CloudLoginFailure::Denied),
        }),
    )
    .await;
    assert_eq!(
        (
            h.store.token(SERVER),
            h.api.exchanges.lock().expect("exchanges").len()
        ),
        (None, 0)
    );
}

/// Nobody approving within the time allowed closes the listener.
#[tokio::test(flavor = "multi_thread")]
async fn an_unanswered_sign_in_times_out_and_stops_listening() {
    let timing = Timing {
        login_timeout: Duration::from_millis(200),
        ..Timing::default()
    };
    let h = harness(Some(SERVER), None, BrowserAction::Record, timing);
    h.service.start_login().await.expect("start");
    let (_, redirect, _) = opened(&h);
    eventually(
        &h.service,
        configured(CloudSession::SignedOut {
            failure: Some(CloudLoginFailure::TimedOut),
        }),
    )
    .await;
    assert!(send_redirect(&redirect, "code=late&state=x").await.is_err());
}

/// Cancelling in Desktop releases the listener; a later redirect reaches nothing.
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_releases_the_listener() {
    let h = harness(Some(SERVER), None, BrowserAction::Record, Timing::default());
    h.service.start_login().await.expect("start");
    let (url, redirect, _) = opened(&h);
    let (_, state) = fakes::redirect_and_state(&url);
    assert_eq!(
        h.service.cancel_login().await,
        configured(CloudSession::SignedOut { failure: None })
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        send_redirect(&redirect, &format!("code=late&state={state}"))
            .await
            .is_err()
    );
    assert_eq!(h.api.exchanges.lock().expect("exchanges").len(), 0);
}

/// A second sign-in replaces the first: only the newest transaction can complete.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_sign_in_replaces_the_previous_one() {
    let h = harness(Some(SERVER), None, BrowserAction::Record, Timing::default());
    h.service.start_login().await.expect("first");
    let (first_url, first_redirect, _) = opened(&h);
    h.service.start_login().await.expect("second");
    let (second_url, second_redirect, _) = opened(&h);
    let (_, first_state) = fakes::redirect_and_state(&first_url);
    let (_, second_state) = fakes::redirect_and_state(&second_url);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        send_redirect(&first_redirect, &format!("code=old&state={first_state}"))
            .await
            .is_err()
    );
    // The old state on the new listener is a stray: 404, and the transaction keeps waiting.
    assert_eq!(
        send_redirect(&second_redirect, &format!("code=old&state={first_state}"))
            .await
            .expect("stray"),
        "HTTP/1.1 404 Not Found"
    );
    send_redirect(&second_redirect, &format!("code=new&state={second_state}"))
        .await
        .expect("redirect");
    eventually(
        &h.service,
        configured(CloudSession::SignedIn { account: alice() }),
    )
    .await;
    let codes: Vec<String> = h
        .api
        .exchanges
        .lock()
        .expect("exchanges")
        .iter()
        .map(|e| e.0.clone())
        .collect();
    assert_eq!(codes, vec!["new".to_owned()]);
}

/// A rejected token is deleted and the person is asked to sign in again.
#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_token_is_deleted() {
    let h = harness(
        Some(SERVER),
        Some("ora_dt_old"),
        BrowserAction::Record,
        Timing::default(),
    );
    *h.api.me.lock().expect("me") = Err(CallError::Rejected);
    h.service.window_focused();
    eventually(&h.service, configured(CloudSession::NeedsSignIn)).await;
    assert_eq!(h.store.token(SERVER), None);
}

/// An unreachable or failing server keeps the token for later.
#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_server_keeps_the_token() {
    let h = harness(
        Some(SERVER),
        Some("ora_dt_old"),
        BrowserAction::Record,
        Timing::default(),
    );
    *h.api.me.lock().expect("me") = Err(CallError::Unreachable("503".to_owned()));
    eventually(
        &h.service,
        configured(CloudSession::Unreachable { account: alice() }),
    )
    .await;
    assert_eq!(h.store.token(SERVER), Some("ora_dt_old".to_owned()));
}

/// Without a usable secure store Cloud is off, and a token is never kept anywhere else.
#[tokio::test(flavor = "multi_thread")]
async fn an_unusable_secure_store_turns_cloud_off() {
    let h = harness(
        Some(SERVER),
        None,
        BrowserAction::Approve("code-1"),
        Timing::default(),
    );
    let ready = h.service.connection().await;
    assert_eq!(ready, configured(CloudSession::SignedOut { failure: None }));
    *h.store.failure.lock().expect("failure") = Some("no secret service".to_owned());
    h.service.start_login().await.expect("start");
    eventually(
        &h.service,
        configured(CloudSession::StorageUnavailable {
            reason: "no secret service".to_owned(),
        }),
    )
    .await;
    assert_eq!(h.store.entries.lock().expect("entries").len(), 0);

    // A store failing at startup is reported as such rather than as signed out.
    let failing = MemoryStore::default();
    *failing.failure.lock().expect("failure") = Some("locked".to_owned());
    let setting = MemorySetting::default();
    *setting.origin.lock().expect("origin") = Some(SERVER.to_owned());
    let service: TestService = CloudConnectionService::start(
        Ports {
            store: failing,
            api: ScriptedApi::new(alice()),
            browser: FakeBrowser::new(BrowserAction::Record),
            setting,
            observer: RecordingObserver::default(),
        },
        Timing::default(),
        Handle::current(),
    );
    assert_eq!(
        service.connection().await,
        configured(CloudSession::StorageUnavailable {
            reason: "locked".to_owned()
        })
    );
}

/// Changing the server cancels the sign-in state, deletes and revokes the old token.
#[tokio::test(flavor = "multi_thread")]
async fn changing_the_server_signs_out_of_the_old_one() {
    let h = harness(
        Some(SERVER),
        Some("ora_dt_old"),
        BrowserAction::Record,
        Timing::default(),
    );
    eventually(
        &h.service,
        configured(CloudSession::SignedIn { account: alice() }),
    )
    .await;
    let next = h
        .service
        .set_server(Some("https://Other.Example/".to_owned()))
        .await
        .expect("switch");
    assert_eq!(
        next,
        CloudConnection::Configured {
            server: "https://other.example".to_owned(),
            session: CloudSession::SignedOut { failure: None },
        }
    );
    assert_eq!(h.store.token(SERVER), None);
    assert!(
        h.api
            .calls
            .lock()
            .expect("calls")
            .contains(&format!("logout {SERVER} ora_dt_old"))
    );
    assert_eq!(
        *h.setting.origin.lock().expect("origin"),
        Some("https://other.example".to_owned())
    );
    assert_eq!(
        h.service.set_server(None).await.expect("forget"),
        CloudConnection::NotConfigured
    );
    assert!(
        h.service
            .set_server(Some("http://cloud.example".to_owned()))
            .await
            .is_err()
    );
}

/// Signing out while the server is unreachable still forgets the token here and says the server
/// did not confirm it.
#[tokio::test(flavor = "multi_thread")]
async fn signing_out_offline_is_local_and_unconfirmed() {
    let h = harness(
        Some(SERVER),
        Some("ora_dt_old"),
        BrowserAction::Record,
        Timing::default(),
    );
    eventually(
        &h.service,
        configured(CloudSession::SignedIn { account: alice() }),
    )
    .await;
    *h.api.logout.lock().expect("logout") = Err(CallError::Unreachable("offline".to_owned()));
    let response = h.service.logout().await;
    assert_eq!(
        (
            response.connection,
            response.revocation,
            h.store.token(SERVER)
        ),
        (
            configured(CloudSession::SignedOut { failure: None }),
            CloudRevocation::Unconfirmed,
            None
        )
    );
    *h.api.logout.lock().expect("logout") = Ok(());
    assert_eq!(
        h.service.logout().await.revocation,
        CloudRevocation::Confirmed
    );
}

/// Focus re-checks are throttled; startup already counts as one check.
#[tokio::test(flavor = "multi_thread")]
async fn focus_checks_are_throttled() {
    let h = harness(
        Some(SERVER),
        Some("ora_dt_old"),
        BrowserAction::Record,
        Timing::default(),
    );
    eventually(
        &h.service,
        configured(CloudSession::SignedIn { account: alice() }),
    )
    .await;
    h.service.window_focused();
    h.service.window_focused();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let checks = h
        .api
        .calls
        .lock()
        .expect("calls")
        .iter()
        .filter(|call| call.starts_with("me "))
        .count();
    assert_eq!(checks, 1);
}

/// A browser that cannot start leaves the connection as it was.
#[tokio::test(flavor = "multi_thread")]
async fn a_browser_that_cannot_start_changes_nothing() {
    let h = harness(Some(SERVER), None, BrowserAction::Fail, Timing::default());
    assert!(h.service.start_login().await.is_err());
    assert_eq!(
        h.service.connection().await,
        configured(CloudSession::SignedOut { failure: None })
    );
    let unconfigured = harness(None, None, BrowserAction::Record, Timing::default());
    assert!(unconfigured.service.start_login().await.is_err());
}
