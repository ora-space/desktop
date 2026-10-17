//! A local stand-in for the platform Git Server: smart HTTP over TLS that requires a client
//! certificate, backed by the real `git http-backend`.
//!
//! Tests point Node code at it exactly as at the real Git Server: an `https` repository URL, a CA
//! that trusts the server, and a git-access client identity. Like the real server it refuses
//! connections without a client certificate, rejects non-fast-forward updates of Workspace
//! branches and any update of an existing delivery ref with `ora: push rejected`, and it can be
//! told to answer with an HTTP status (403, 503, ...) or to stop answering for a while.
// A fixture that cannot set itself up has no better answer than failing the test loudly.
#![allow(clippy::expect_used)]

use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

/// The update rules the real Git Server enforces for sandbox writes, as a `pre-receive` hook.
const PRE_RECEIVE: &str = r#"#!/bin/sh
zero=0000000000000000000000000000000000000000
while read old new ref; do
  case "$ref" in
    refs/ora/deliveries/*)
      [ "$old" = "$zero" ] || { echo "ora: push rejected" >&2; exit 1; } ;;
    refs/heads/workspaces/*)
      if [ "$old" != "$zero" ] && ! git merge-base --is-ancestor "$old" "$new"; then
        echo "ora: push rejected" >&2; exit 1
      fi ;;
    *) echo "ora: push rejected" >&2; exit 1 ;;
  esac
done
"#;

/// Paths of the material a Node needs to talk to this server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitAccessMaterial {
    pub ca_cert: PathBuf,
    pub client_cert: PathBuf,
    pub client_key: PathBuf,
}

/// One request the server answered, as far as tests need to see it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServedRequest {
    pub method: String,
    /// The URL path without the query.
    pub path: String,
    /// The `Ora-Execution-Id` header, if the client sent one.
    pub execution_id: Option<String>,
    pub status: u16,
}

#[derive(Default)]
struct Behavior {
    /// Statuses answered to the next requests, in order, before any real handling.
    queued: VecDeque<u16>,
    /// Status answered to every request while set.
    always: Option<u16>,
    served: Vec<ServedRequest>,
}

/// A running server; dropping it stops the listener.
pub struct GitServer {
    root: PathBuf,
    port: u16,
    access: GitAccessMaterial,
    behavior: Arc<Mutex<Behavior>>,
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl GitServer {
    /// Issues a server and a client identity under `directory` and starts listening on an
    /// ephemeral loopback port. Repositories live under `directory/repositories`.
    pub fn start(directory: &Path) -> Self {
        let root = directory.join("repositories");
        fs::create_dir_all(&root).expect("create repository root");
        let (server_config, access) = issue(directory);
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind Git Server");
        let port = listener.local_addr().expect("local address").port();
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let behavior = Arc::new(Mutex::new(Behavior::default()));
        let paused = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let (behavior, paused, stop, root) =
                (behavior.clone(), paused.clone(), stop.clone(), root.clone());
            let config = Arc::new(server_config);
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    if paused.load(Ordering::SeqCst) {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    match listener.accept() {
                        Ok((socket, _)) => {
                            let (config, behavior, root) =
                                (config.clone(), behavior.clone(), root.clone());
                            thread::spawn(move || {
                                let _ = serve(socket, &config, &root, &behavior);
                            });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            root,
            port,
            access,
            behavior,
            paused,
            stop,
            worker: Some(worker),
        }
    }

    /// The git-access identity and the CA that trusts this server.
    pub fn access(&self) -> &GitAccessMaterial {
        &self.access
    }

    /// Creates an empty bare repository at `name` (for example `tenant/project.git`) with the
    /// Git Server's update rules and returns its directory.
    pub fn create_repository(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(&path).expect("create repository directory");
        git(&path, &["init", "--bare", "--quiet", "--template="]);
        git(&path, &["config", "http.receivepack", "true"]);
        let hooks = path.join("hooks");
        fs::create_dir_all(&hooks).expect("create hooks");
        let hook = hooks.join("pre-receive");
        fs::write(&hook, PRE_RECEIVE).expect("write pre-receive");
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("hook mode");
        path
    }

    /// The `https` URL of repository `name`.
    pub fn url(&self, name: &str) -> String {
        format!("https://localhost:{}/{name}", self.port)
    }

    /// Answers the next `count` requests with `status` before handling any further request.
    pub fn fail_next(&self, status: u16, count: usize) {
        let mut behavior = self.lock();
        behavior.queued.extend(std::iter::repeat_n(status, count));
    }

    /// Answers every request with `status` until cleared with `None`.
    pub fn answer_always(&self, status: Option<u16>) {
        self.lock().always = status;
    }

    /// Stops accepting connections while `paused` is set; clients wait until it is cleared.
    pub fn pause(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }

    /// The flag behind [`GitServer::pause`], for a test thread that releases it later.
    pub fn pause_flag(&self) -> Arc<AtomicBool> {
        self.paused.clone()
    }

    /// Every request answered so far, in order.
    pub fn served(&self) -> Vec<ServedRequest> {
        self.lock().served.clone()
    }

    /// Locks the shared behavior, surviving a panicked handler.
    fn lock(&self) -> std::sync::MutexGuard<'_, Behavior> {
        self.behavior.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for GitServer {
    /// Stops the accept loop; handlers end with their bounded socket timeouts.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Runs a Git command in `directory` for fixture setup and panics on failure.
pub fn git(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Issues a CA, a `localhost` server certificate and a clientAuth leaf, writes the client-side
/// material, and builds a TLS configuration that requires a client certificate from that CA.
fn issue(directory: &Path) -> (rustls::ServerConfig, GitAccessMaterial) {
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("CA params");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
    ];
    let ca =
        rcgen::CertifiedIssuer::self_signed(ca_params, rcgen::KeyPair::generate().expect("CA key"))
            .expect("self-signed CA");
    let server_key = rcgen::KeyPair::generate().expect("server key");
    let mut server_params =
        rcgen::CertificateParams::new(vec!["localhost".to_owned()]).expect("server params");
    server_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let server = server_params
        .signed_by(&server_key, &ca)
        .expect("server certificate");
    let client_key = rcgen::KeyPair::generate().expect("client key");
    let mut client_params =
        rcgen::CertificateParams::new(Vec::<String>::new()).expect("client params");
    client_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
    let client = client_params
        .signed_by(&client_key, &ca)
        .expect("client certificate");
    let material = directory.join("git-access");
    fs::create_dir_all(&material).expect("material directory");
    let access = GitAccessMaterial {
        ca_cert: material.join("ca.pem"),
        client_cert: material.join("client.pem"),
        client_key: material.join("client-key.pem"),
    };
    fs::write(&access.ca_cert, ca.pem()).expect("write CA");
    fs::write(&access.client_cert, client.pem()).expect("write client certificate");
    fs::write(&access.client_key, client_key.serialize_pem()).expect("write client key");
    fs::set_permissions(&access.client_key, fs::Permissions::from_mode(0o600))
        .expect("client key mode");
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca.der().clone()).expect("trust CA");
    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .expect("client verifier");
    let config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![server.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(server_key.serialize_der()).into(),
        )
        .expect("server TLS configuration");
    (config, access)
}

/// A parsed request: method, path, query, headers and the decoded body.
struct Request {
    method: String,
    path: String,
    query: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    /// The first header named `name`, ignoring case.
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Answers one connection: the TLS handshake fails without a client certificate.
fn serve(
    socket: TcpStream,
    config: &Arc<rustls::ServerConfig>,
    root: &Path,
    behavior: &Mutex<Behavior>,
) -> std::io::Result<()> {
    socket.set_nonblocking(false)?;
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    socket.set_write_timeout(Some(Duration::from_secs(10)))?;
    let connection =
        rustls::ServerConnection::new(config.clone()).map_err(std::io::Error::other)?;
    let mut stream = rustls::StreamOwned::new(connection, socket);
    let request = read_request(&mut stream)?;
    let injected = {
        let mut behavior = behavior.lock().unwrap_or_else(PoisonError::into_inner);
        behavior.queued.pop_front().or(behavior.always)
    };
    let (status, head, body) = match injected {
        Some(status) => (status, String::new(), Vec::new()),
        None => backend(root, &request)?,
    };
    behavior
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .served
        .push(ServedRequest {
            method: request.method.clone(),
            path: request.path.clone(),
            execution_id: request.header("Ora-Execution-Id").map(str::to_owned),
            status,
        });
    write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\n{head}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    stream.flush()?;
    stream.conn.send_close_notify();
    stream.flush()
}

/// Reads the request line, headers and a `Content-Length` or chunked body.
fn read_request(stream: &mut impl Read) -> std::io::Result<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let (path, query) = target
        .split_once('?')
        .map_or((target.clone(), String::new()), |(path, query)| {
            (path.to_owned(), query.to_owned())
        });
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" {
            break;
        }
        if let Some((key, value)) = header.trim_end().split_once(':') {
            headers.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    let mut request = Request {
        method,
        path,
        query,
        headers,
        body: Vec::new(),
    };
    if request
        .header("Transfer-Encoding")
        .is_some_and(|value| value.eq_ignore_ascii_case("chunked"))
    {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size)?;
            let size = usize::from_str_radix(size.trim(), 16).map_err(std::io::Error::other)?;
            let mut chunk = vec![0; size + 2];
            reader.read_exact(&mut chunk)?;
            if size == 0 {
                break;
            }
            request.body.extend_from_slice(&chunk[..size]);
        }
    } else if let Some(length) = request.header("Content-Length") {
        let length = length.parse().map_err(std::io::Error::other)?;
        request.body = vec![0; length];
        reader.read_exact(&mut request.body)?;
    }
    Ok(request)
}

/// Runs `git http-backend` as a CGI program for the request and splits its output.
fn backend(root: &Path, request: &Request) -> std::io::Result<(u16, String, Vec<u8>)> {
    let mut command = Command::new("git");
    command
        .arg("http-backend")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REMOTE_USER", "sandbox")
        .env("REMOTE_ADDR", "127.0.0.1")
        .env("PATH_INFO", &request.path)
        .env("REQUEST_METHOD", &request.method)
        .env("QUERY_STRING", &request.query)
        .env("CONTENT_LENGTH", request.body.len().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (header, variable) in [
        ("Content-Type", "CONTENT_TYPE"),
        ("Content-Encoding", "HTTP_CONTENT_ENCODING"),
        ("Git-Protocol", "GIT_PROTOCOL"),
    ] {
        if let Some(value) = request.header(header) {
            command.env(variable, value);
        }
    }
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let body = request.body.clone();
    let writer = thread::spawn(move || stdin.write_all(&body));
    let output = child.wait_with_output()?;
    let _ = writer.join();
    let split = output
        .stdout
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|at| (at, 4))
        .or_else(|| {
            output
                .stdout
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|at| (at, 2))
        })
        .unwrap_or((output.stdout.len(), 0));
    let head = String::from_utf8_lossy(&output.stdout[..split.0]).into_owned();
    let body = output.stdout[split.0 + split.1..].to_vec();
    let mut status = 200;
    let mut headers = String::new();
    for line in head.lines().filter(|line| !line.trim().is_empty()) {
        match line.split_once(':') {
            Some((key, value)) if key.eq_ignore_ascii_case("Status") => {
                status = value
                    .split_whitespace()
                    .next()
                    .and_then(|code| code.parse().ok())
                    .unwrap_or(500);
            }
            Some(_) => {
                headers.push_str(line.trim_end());
                headers.push_str("\r\n");
            }
            None => {}
        }
    }
    Ok((status, headers, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Runs Git as a client of `server`, with or without the git-access identity.
    fn client(server: &GitServer, directory: &Path, identity: bool, args: &[&str]) -> bool {
        let access = server.access();
        let mut command = Command::new("git");
        command
            .current_dir(directory)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .arg("-c")
            .arg(format!("http.sslCAInfo={}", access.ca_cert.display()));
        if identity {
            command
                .arg("-c")
                .arg(format!("http.sslCert={}", access.client_cert.display()))
                .arg("-c")
                .arg(format!("http.sslKey={}", access.client_key.display()));
        }
        command
            .args(["-c", "http.extraHeader=Ora-Execution-Id: execution-1"])
            .args(args)
            .output()
            .expect("run git client")
            .status
            .success()
    }

    /// The double behaves like the Git Server for the operations Node code performs.
    #[test]
    fn serves_fetch_and_push_only_to_clients_with_the_access_identity() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let server = GitServer::start(directory.path());
        server.create_repository("tenant/project.git");
        let url = server.url("tenant/project.git");
        let work = directory.path().join("work");
        fs::create_dir(&work).expect("work directory");
        git(&work, &["init", "--quiet", "-b", "main"]);
        git(
            &work,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.test",
                "commit",
                "--allow-empty",
                "-qm",
                "one",
            ],
        );
        let first = git(&work, &["rev-parse", "HEAD"]);
        let branch = "refs/heads/workspaces/w/main";
        assert!(!client(
            &server,
            &work,
            false,
            &["push", &url, &format!("HEAD:{branch}")]
        ));
        assert!(client(
            &server,
            &work,
            true,
            &["push", &url, &format!("HEAD:{branch}")]
        ));
        git(
            &work,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.test",
                "commit",
                "--allow-empty",
                "-qm",
                "two",
            ],
        );
        assert!(client(
            &server,
            &work,
            true,
            &["push", &url, &format!("HEAD:{branch}")]
        ));
        let second = git(&work, &["rev-parse", "HEAD"]);
        // Rewriting the pushed branch is refused like the real server's fast-forward rule.
        git(&work, &["reset", "--quiet", "--hard", "HEAD~1"]);
        git(
            &work,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.test",
                "commit",
                "--allow-empty",
                "-qm",
                "other",
            ],
        );
        assert!(!client(
            &server,
            &work,
            true,
            &["push", "--force", &url, &format!("HEAD:{branch}")]
        ));
        server.fail_next(503, 1);
        assert!(!client(&server, &work, true, &["ls-remote", &url]));
        assert!(client(
            &server,
            &work,
            true,
            &["fetch", "-q", &url, &format!("{branch}:refs/fetched")]
        ));
        assert_eq!(git(&work, &["rev-parse", "refs/fetched"]), second);
        // Delivery refs are create-only, and nothing outside the sandbox namespaces is writable.
        let delivery = |commit: &str| format!("{commit}:refs/ora/deliveries/d");
        assert!(client(
            &server,
            &work,
            true,
            &["push", &url, &delivery(&first)]
        ));
        assert!(!client(
            &server,
            &work,
            true,
            &["push", "--force", &url, &delivery(&second)]
        ));
        assert!(!client(
            &server,
            &work,
            true,
            &["push", &url, "HEAD:refs/heads/main"]
        ));
        assert!(
            server
                .served()
                .iter()
                .all(|request| request.execution_id.as_deref() == Some("execution-1"))
        );
    }
}
