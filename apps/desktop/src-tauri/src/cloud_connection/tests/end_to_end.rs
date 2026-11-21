//! The real HTTP client, loopback listener and coordinator against a stub Cloud server.

use std::sync::{Arc, Mutex};

use ora_contracts::{CloudConnection, CloudRevocation, CloudSession};
use pretty_assertions::assert_eq;
use tokio::runtime::Handle;

use super::super::api::HttpCloudApi;
use super::super::ports::BrowserOpener;
use super::super::service::{CloudConnectionService, Ports, Timing};
use super::fakes::{MemorySetting, MemoryStore, RecordingObserver, redirect_and_state};
use super::stub_cloud::StubCloud;
use super::{alice, send_redirect};

/// A browser that approves on the stub, the way the Cloud page would after the person clicks.
#[derive(Clone)]
struct ApprovingBrowser {
    cloud: Arc<StubCloud>,
    runtime: Handle,
    opened: Arc<Mutex<Vec<String>>>,
}

impl BrowserOpener for ApprovingBrowser {
    /// Approves with the challenge from the address and follows the redirect.
    fn open(&self, url: &str) -> Result<(), String> {
        self.opened.lock().expect("opened").push(url.to_owned());
        let parsed = url::Url::parse(url).map_err(|error| error.to_string())?;
        let challenge = parsed
            .query_pairs()
            .find(|(key, _)| key == "code_challenge")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default();
        let (redirect, state) = redirect_and_state(url);
        let query = self.cloud.approve(&redirect, &state, &challenge);
        self.runtime
            .spawn(async move { send_redirect(&redirect, &query).await });
        Ok(())
    }
}

/// Sign in, survive a restart, notice revocation, sign out: all over real sockets.
#[tokio::test(flavor = "multi_thread")]
async fn signs_in_restarts_notices_revocation_and_signs_out() {
    let cloud = Arc::new(StubCloud::start().await);
    let store = MemoryStore::default();
    let setting = MemorySetting::default();
    let start = || {
        CloudConnectionService::start(
            Ports {
                store: store.clone(),
                api: HttpCloudApi,
                browser: ApprovingBrowser {
                    cloud: Arc::clone(&cloud),
                    runtime: Handle::current(),
                    opened: Arc::default(),
                },
                setting: setting.clone(),
                observer: RecordingObserver::default(),
            },
            Timing::default(),
            Handle::current(),
        )
    };
    let signed_in = CloudConnection::Configured {
        server: cloud.origin.clone(),
        session: CloudSession::SignedIn { account: alice() },
    };

    let service = start();
    service
        .set_server(Some(cloud.origin.clone()))
        .await
        .expect("server");
    service.start_login().await.expect("start");
    super::eventually_for(&service, signed_in.clone()).await;
    assert_eq!(store.token(&cloud.origin), Some("ora_dt_stub1".to_owned()));

    // A restart restores the stored token and confirms it with the server.
    let restarted = start();
    super::eventually_for(&restarted, signed_in.clone()).await;

    // Revoked on the server: the next check deletes the token and asks for a new sign-in.
    cloud.revoke_all();
    let after_revocation = start();
    super::eventually_for(
        &after_revocation,
        CloudConnection::Configured {
            server: cloud.origin.clone(),
            session: CloudSession::NeedsSignIn,
        },
    )
    .await;
    assert_eq!(store.token(&cloud.origin), None);

    // Sign in again and sign out: the server forgets the token too.
    after_revocation.start_login().await.expect("again");
    super::eventually_for(&after_revocation, signed_in).await;
    assert_eq!(cloud.live_tokens(), 1);
    let response = after_revocation.logout().await;
    assert_eq!(
        (
            response.revocation,
            cloud.live_tokens(),
            store.token(&cloud.origin)
        ),
        (CloudRevocation::Confirmed, 0, None)
    );
}
