//! Synthetic mTLS gateway shared by the Node's grant and real plugin-session tests.
use super::ModelProxyConfig;
use serde_json::{Value, json};
use std::time::Duration;
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
    pub(crate) foreign_endpoint: AtomicBool,
    pub(crate) deny_renewal: AtomicBool,
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
