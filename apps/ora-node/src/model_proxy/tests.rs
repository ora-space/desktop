#![allow(clippy::unwrap_used)]

use super::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// Generates narrowly purposed TLS material and serves the real grant HTTP contract over mTLS.
pub(crate) struct Gateway {
    pub(crate) config: ModelProxyConfig,
    pub(crate) requests: Arc<Mutex<Vec<(String, Value)>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    pub(crate) behavior: Arc<GatewayBehavior>,
}

#[derive(Default)]
pub(crate) struct GatewayBehavior {
    foreign_endpoint: AtomicBool,
    deny_renewal: AtomicBool,
    pub(crate) ending_initially: AtomicBool,
    pub(crate) ending_at_renewal: AtomicBool,
}

impl Gateway {
    /// Test tokens are synthetic and never pass through the durable execution protocol.
    pub(crate) fn new(root: &std::path::Path) -> Self {
        ora_logging::initialize_test_clock();
        let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
        let ca =
            rcgen::CertifiedIssuer::self_signed(ca_params, rcgen::KeyPair::generate().unwrap())
                .unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let server = params.signed_by(&key, &ca).unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(ca.der().clone()).unwrap();
        let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .unwrap();
        let server_config = Arc::new(
            rustls::ServerConfig::builder()
                .with_client_cert_verifier(verifier)
                .with_single_cert(
                    vec![server.der().clone()],
                    rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
                )
                .unwrap(),
        );
        let client_key = rcgen::KeyPair::generate().unwrap();
        let mut client_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        client_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        let client = client_params.signed_by(&client_key, &ca).unwrap();
        let ca_cert = root.join("model-ca.pem");
        let client_cert = root.join("model-client.pem");
        let client_key_path = root.join("model-client-key.pem");
        std::fs::write(&ca_cert, ca.pem()).unwrap();
        std::fs::write(&client_cert, client.pem()).unwrap();
        std::fs::write(&client_key_path, client_key.serialize_pem()).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let behavior = Arc::new(GatewayBehavior::default());
        let serving = behavior.clone();
        let worker = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(/*secs*/ 2)))
                            .unwrap();
                        socket
                            .set_write_timeout(Some(Duration::from_secs(/*secs*/ 2)))
                            .unwrap();
                        let connection =
                            rustls::ServerConnection::new(server_config.clone()).unwrap();
                        let mut stream = rustls::StreamOwned::new(connection, socket);
                        let _ = serve(&mut stream, &observed, port, &serving);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(/*millis*/ 5))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            config: ModelProxyConfig {
                gateway_url: format!("https://localhost:{port}"),
                ca_cert,
                client_cert,
                client_key: client_key_path,
            },
            requests,
            stop,
            worker: Some(worker),
            behavior,
        }
    }
}

impl Drop for Gateway {
    /// Fixture timeouts ensure a failed assertion cannot leave a listener running.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

/// Captures only authenticated scoped grant requests and returns short renewable fixture grants.
fn serve(
    stream: &mut impl ReadWrite,
    observed: &Mutex<Vec<(String, Value)>>,
    port: u16,
    behavior: &GatewayBehavior,
) -> std::io::Result<()> {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") && header.len() < 8192 {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        header.extend(byte);
    }
    let header = String::from_utf8(header).unwrap();
    let request = header.lines().next().unwrap().to_string();
    let length: usize = header
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .unwrap()
        .parse()
        .unwrap();
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    let body: Value = serde_json::from_slice(&body).unwrap();
    observed.lock().unwrap().push((request.clone(), body));
    if request.starts_with("POST /internal/v1/model-grants ")
        && behavior.ending_initially.load(Ordering::SeqCst)
        || request.contains("/renew ") && behavior.ending_at_renewal.load(Ordering::SeqCst)
    {
        let body =
            json!({"code":"model_session_ending","params":{},"requestId":"fixture"}).to_string();
        write!(
            stream,
            "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        return stream.flush();
    }
    if request.starts_with("POST ")
        && request.contains("/renew ")
        && behavior.deny_renewal.load(Ordering::SeqCst)
    {
        stream.write_all(
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?;
        return stream.flush();
    }
    let data_url = if behavior.foreign_endpoint.load(Ordering::SeqCst) {
        "https://untrusted.invalid/runtime/openai/v1".to_owned()
    } else {
        format!("https://localhost:{port}/runtime/openai/v1")
    };
    let response = if request.starts_with("POST /internal/v1/model-grants ") {
        json!({"grantId":"grant-1", "token":"synthetic-temporary-token", "expiresAt": chrono::Local::now().fixed_offset() + chrono::Duration::seconds(2),
            "protocol":"openai-completions", "proxyBaseUrl":data_url,
            "model":{"id":"team/model", "name":"Fixture", "contextWindow":32768,"maxTokens":1024}})
    } else if request.starts_with("POST ") {
        json!({"expiresAt":chrono::Local::now().fixed_offset() + chrono::Duration::minutes(15)})
    } else {
        json!({})
    }.to_string();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    )?;
    stream.flush()
}

/// Allows the synchronous fixture to operate on a TLS stream without exposing transport internals.
trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

#[tokio::test]
async fn renewable_access_keeps_token_in_memory_and_revokes_at_end() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    let mut access = ModelAccess::open(
        gateway.config.clone(),
        &ModelBindingId::new("binding-1"),
        &ExecutionId::new("execution-1"),
        root.path(),
    )
    .await
    .unwrap();
    let home = PathBuf::from(&access.environment["HOME"]);
    assert_eq!(
        access
            .environment
            .get("OPENCODE_DISABLE_PROJECT_CONFIG")
            .map(String::as_str),
        Some("true")
    );
    let config: Value =
        serde_json::from_str(&access.environment["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(config["model"], "ora-model/team/model");
    assert_eq!(
        config["provider"]["ora-model"]["options"]["apiKey"],
        "{env:ORA_MODEL_ACCESS_TOKEN}"
    );
    assert!(!config.to_string().contains("synthetic-temporary-token"));
    tokio::time::timeout(Duration::from_secs(/*secs*/ 4), async {
        while gateway.requests.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(/*millis*/ 10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        access.environment["ORA_MODEL_ACCESS_TOKEN"],
        "synthetic-temporary-token"
    );
    access.close().await;
    drop(access);
    assert!(!home.exists());
    assert_eq!(
        *gateway.requests.lock().unwrap(),
        vec![
            (
                "POST /internal/v1/model-grants HTTP/1.1".to_string(),
                json!({"bindingId":"binding-1","executionId":"execution-1"})
            ),
            (
                "POST /internal/v1/model-grants/grant-1/renew HTTP/1.1".to_string(),
                json!({"executionId":"execution-1"})
            ),
            (
                "DELETE /internal/v1/model-grants/grant-1 HTTP/1.1".to_string(),
                json!({"executionId":"execution-1"})
            ),
        ]
    );
}

#[tokio::test]
async fn foreign_data_endpoint_is_rejected_and_grant_revoked() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    gateway
        .behavior
        .foreign_endpoint
        .store(true, Ordering::SeqCst);
    let result = ModelAccess::open(
        gateway.config.clone(),
        &ModelBindingId::new("binding-1"),
        &ExecutionId::new("execution-1"),
        root.path(),
    )
    .await;
    let Err(error) = result else {
        panic!("foreign endpoint must not receive a token")
    };
    assert_eq!(error.to_string(), "model_grant_invalid");
    assert_eq!(
        gateway
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|(route, _)| route.clone())
            .collect::<Vec<_>>(),
        vec![
            "POST /internal/v1/model-grants HTTP/1.1",
            "DELETE /internal/v1/model-grants/grant-1 HTTP/1.1"
        ]
    );
}

#[tokio::test]
async fn denied_renewal_notifies_the_conversation_to_stop() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    gateway.behavior.deny_renewal.store(true, Ordering::SeqCst);
    let mut access = ModelAccess::open(
        gateway.config.clone(),
        &ModelBindingId::new("binding-1"),
        &ExecutionId::new("execution-1"),
        root.path(),
    )
    .await
    .unwrap();
    let status = tokio::time::timeout(
        Duration::from_secs(/*secs*/ 4),
        access
            .status
            .wait_for(|status| *status != ModelAccessStatus::Active),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(*status, ModelAccessStatus::Revoked);
    drop(status);
    access.close().await;
    assert_eq!(gateway.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn close_cancels_the_scheduled_renewal() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    let mut access = ModelAccess::open(
        gateway.config.clone(),
        &ModelBindingId::new("binding-1"),
        &ExecutionId::new("execution-1"),
        root.path(),
    )
    .await
    .unwrap();
    access.close().await;
    // This crosses the fixture grant's expiry, so an unowned timer would have renewed by now.
    tokio::time::sleep(Duration::from_millis(/*millis*/ 2200)).await;
    assert_eq!(gateway.requests.lock().unwrap().len(), 2);
}

#[test]
fn anthropic_configuration_preserves_model_ids_and_isolates_cli_storage() {
    let root = tempfile::tempdir().unwrap();
    let grant = Grant {
        grant_id: "grant-1".into(),
        token: "synthetic-token".into(),
        expires_at: chrono::Local::now().fixed_offset(),
        protocol: Protocol::AnthropicMessages,
        proxy_base_url: "https://gateway/runtime/anthropic/v1".into(),
        model: Model {
            id: "team/model".into(),
            name: "Model".into(),
            context_window: 200000,
            max_tokens: 8192,
        },
    };
    let (environment, home) =
        opencode::environment(&grant, &root.path().join("ca.pem"), root.path()).unwrap();
    let config: Value = serde_json::from_str(&environment["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(config["provider"]["ora-model"]["npm"], "@ai-sdk/anthropic");
    assert_eq!(
        config["provider"]["ora-model"]["models"]["team/model"],
        json!({"name":"Model","limit":{"context":200000,"output":8192}})
    );
    for key in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
    ] {
        assert!(PathBuf::from(&environment[key]).starts_with(home.path()));
    }
    assert!(!config.to_string().contains("synthetic-token"));
}

#[test]
fn expired_and_excessive_grant_lifetimes_are_rejected() {
    ora_logging::initialize_test_clock();
    assert!(
        renewal_delay(chrono::Local::now().fixed_offset() - chrono::Duration::seconds(1)).is_err()
    );
    assert!(
        renewal_delay(chrono::Local::now().fixed_offset() + chrono::Duration::hours(1)).is_err()
    );
}

/// A CA file published to a workload must not contain a combined private-key PEM block.
#[test]
fn public_ca_material_rejects_private_key_blocks_before_grant_creation() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    let mut combined = std::fs::read(&gateway.config.ca_cert).unwrap();
    combined.extend(std::fs::read(&gateway.config.client_key).unwrap());
    std::fs::write(&gateway.config.ca_cert, combined).unwrap();
    let error = gateway.config.validate().unwrap_err();
    assert_eq!(error.to_string(), "model_proxy_tls_invalid");
    assert!(gateway.requests.lock().unwrap().is_empty());
}

#[test]
fn runtime_state_refuses_existing_files_and_symlinks_without_changing_them() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let grant = Grant {
        grant_id: "grant-1".into(),
        token: "synthetic-token".into(),
        expires_at: chrono::Local::now().fixed_offset(),
        protocol: Protocol::OpenaiCompletions,
        proxy_base_url: "https://gateway/runtime/openai/v1".into(),
        model: Model {
            id: "model".into(),
            name: "Model".into(),
            context_window: 32768,
            max_tokens: 1024,
        },
    };
    let runtime = root.path().join("model-runtime");
    std::fs::write(&runtime, b"existing user data").unwrap();
    assert!(opencode::environment(&grant, &root.path().join("ca.pem"), root.path()).is_err());
    assert_eq!(std::fs::read(&runtime).unwrap(), b"existing user data");
    std::fs::remove_file(&runtime).unwrap();
    symlink(checkout.path(), &runtime).unwrap();
    assert!(opencode::environment(&grant, &root.path().join("ca.pem"), root.path()).is_err());
    assert_eq!(std::fs::read_dir(checkout.path()).unwrap().count(), 0);
}
