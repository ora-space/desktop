use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

/// Real TLS serves Git's dumb-HTTP repository files, with no widening to file:// in production.
pub struct HttpsRepository {
    pub address: String,
    pub certificate: PathBuf,
    pub reject_auth: Arc<AtomicBool>,
    pub require_credentials: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl HttpsRepository {
    /// Installs a fixture CA and starts a private, bounded HTTPS listener on an ephemeral port.
    pub fn new(root: &Path, repository: PathBuf) -> Self {
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        let ca = rcgen::CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
        params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let cert = params.signed_by(&key, &ca).unwrap();
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
            )
            .unwrap();
        let certificate = root.join("ca.pem");
        fs::write(&certificate, ca.pem()).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let reject_auth = Arc::new(AtomicBool::new(false));
        let require_credentials = Arc::new(AtomicBool::new(false));
        let authenticate = require_credentials.clone();
        let paused = Arc::new(AtomicBool::new(false));
        let (exit, reject, pause) = (stop.clone(), reject_auth.clone(), paused.clone());
        let worker = thread::spawn(move || {
            let config = Arc::new(config);
            while !exit.load(Ordering::SeqCst) {
                if pause.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(/*millis*/ 10));
                    continue;
                }
                match listener.accept() {
                    Ok((socket, _)) => {
                        let _ = serve(
                            socket,
                            &config,
                            &repository,
                            reject.load(Ordering::SeqCst),
                            authenticate.load(Ordering::SeqCst),
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(/*millis*/ 10))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address: format!("https://localhost:{port}/repo.git"),
            certificate,
            reject_auth,
            require_credentials,
            paused,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for HttpsRepository {
    /// Bounded socket timeouts and an explicit stop flag prevent fixture threads leaking on failure.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Returns only files inside the fixture repository, never interpreting request paths as shell input.
fn serve(
    socket: TcpStream,
    config: &Arc<rustls::ServerConfig>,
    root: &Path,
    reject: bool,
    authenticate: bool,
) -> std::io::Result<()> {
    socket.set_read_timeout(Some(Duration::from_secs(/*secs*/ 2)))?;
    socket.set_write_timeout(Some(Duration::from_secs(/*secs*/ 2)))?;
    let connection =
        rustls::ServerConnection::new(config.clone()).map_err(std::io::Error::other)?;
    let mut stream = rustls::StreamOwned::new(connection, socket);
    let mut request = Vec::new();
    while request.len() < 8192 && !request.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        request.extend(byte);
    }
    let authenticated = String::from_utf8_lossy(&request)
        .lines()
        .any(|line| line.eq_ignore_ascii_case("Authorization: Basic Zml4dHVyZTpzZWNyZXQ="));
    if reject || (authenticate && !authenticated) {
        stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
    } else {
        let request = String::from_utf8_lossy(&request);
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|path| path.split('?').next())
            .and_then(|path| path.strip_prefix("/repo.git/"));
        let contents = path
            .filter(|path| {
                Path::new(path)
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
            })
            .and_then(|path| fs::read(root.join(path)).ok());
        if let Some(body) = contents {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(&body)?;
        } else {
            stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )?;
        }
    }
    stream.flush()?;
    stream.conn.send_close_notify();
    stream.flush()
}
