//! A minimal HTTP stand-in for the Cloud server's desktop-login routes, enforcing their contract:
//! a code is spent by its first exchange and yields a token only with the verifier of its
//! challenge and the same redirect URI; the token works until revoked.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Default)]
struct Ledger {
    codes: HashMap<String, (String, String)>,
    tokens: Vec<String>,
    issued: u32,
}

/// The running stub; dropping it leaves the accept task to end with the test runtime.
pub(super) struct StubCloud {
    pub(super) origin: String,
    ledger: Arc<Mutex<Ledger>>,
}

impl StubCloud {
    /// Listens on loopback http, which the host accepts only for loopback servers.
    pub(super) async fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind stub");
        let origin = format!(
            "http://127.0.0.1:{}",
            listener.local_addr().expect("addr").port()
        );
        let ledger = Arc::new(Mutex::new(Ledger::default()));
        let shared = Arc::clone(&ledger);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let ledger = Arc::clone(&shared);
                tokio::spawn(async move { serve(stream, ledger).await });
            }
        });
        Self { origin, ledger }
    }

    /// Plays the approval page: records a code for the challenge and returns the redirect target.
    pub(super) fn approve(&self, redirect_uri: &str, state: &str, challenge: &str) -> String {
        let code = format!(
            "code-{}",
            self.ledger.lock().expect("ledger").codes.len() + 1
        );
        self.ledger.lock().expect("ledger").codes.insert(
            code.clone(),
            (challenge.to_owned(), redirect_uri.to_owned()),
        );
        format!("code={code}&state={state}")
    }

    /// Revokes every token, as disabling the account would.
    pub(super) fn revoke_all(&self) {
        self.ledger.lock().expect("ledger").tokens.clear();
    }

    /// Tokens currently accepted.
    pub(super) fn live_tokens(&self) -> usize {
        self.ledger.lock().expect("ledger").tokens.len()
    }
}

/// Answers one request.
async fn serve(mut stream: TcpStream, ledger: Arc<Mutex<Ledger>>) {
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 4096];
    let (head, body) = loop {
        let read = stream.read(&mut buffer).await.unwrap_or(0);
        if read == 0 {
            return;
        }
        raw.extend_from_slice(&buffer[..read]);
        if let Some(end) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..end]).into_owned();
            let length = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            while raw.len() < end + 4 + length {
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                if read == 0 {
                    break;
                }
                raw.extend_from_slice(&buffer[..read]);
            }
            break (head, raw[end + 4..].to_vec());
        }
    };
    let request_line = head.lines().next().unwrap_or_default().to_owned();
    let bearer = head.lines().find_map(|line| {
        line.strip_prefix("authorization: Bearer ")
            .or_else(|| line.strip_prefix("Authorization: Bearer "))
            .map(str::to_owned)
    });
    let (status, response) = route(&request_line, bearer.as_deref(), &body, &ledger);
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    );
    let _ = stream.write_all(reply.as_bytes()).await;
}

/// The contract of the three routes plus the account check.
fn route(
    request_line: &str,
    bearer: Option<&str>,
    body: &[u8],
    ledger: &Mutex<Ledger>,
) -> (&'static str, String) {
    let mut ledger = ledger.lock().expect("ledger");
    let live =
        |ledger: &Ledger| bearer.is_some_and(|token| ledger.tokens.iter().any(|t| t == token));
    if request_line.starts_with("POST /auth/desktop/token ") {
        let input: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
        let field = |name: &str| input[name].as_str().unwrap_or_default().to_owned();
        let spent = ledger.codes.remove(&field("code"));
        let verified = spent.is_some_and(|(challenge, redirect)| {
            URL_SAFE_NO_PAD.encode(Sha256::digest(field("codeVerifier").as_bytes())) == challenge
                && redirect == field("redirectUri")
        });
        if !verified {
            return ("400 Bad Request", r#"{"code":"invalid_grant"}"#.to_owned());
        }
        ledger.issued += 1;
        let token = format!("ora_dt_stub{}", ledger.issued);
        ledger.tokens.push(token.clone());
        return (
            "200 OK",
            serde_json::json!({"token": token, "user": {"id": "u-alice", "displayName": "Alice"}})
                .to_string(),
        );
    }
    if request_line.starts_with("GET /api/v1/me ") {
        return if live(&ledger) {
            (
                "200 OK",
                r#"{"id":"u-alice","displayName":"Alice"}"#.to_owned(),
            )
        } else {
            (
                "401 Unauthorized",
                r#"{"code":"unauthenticated"}"#.to_owned(),
            )
        };
    }
    if request_line.starts_with("POST /auth/desktop/logout ") {
        let Some(token) = bearer else {
            return (
                "401 Unauthorized",
                r#"{"code":"unauthenticated"}"#.to_owned(),
            );
        };
        ledger.tokens.retain(|t| t != token);
        return ("204 No Content", String::new());
    }
    ("404 Not Found", r#"{"code":"not_found"}"#.to_owned())
}
