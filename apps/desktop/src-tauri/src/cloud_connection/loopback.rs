//! The one-shot loopback listener that receives the browser's redirect (RFC 8252 §7.3).
//!
//! It binds `127.0.0.1` on a port the system picks, never a public interface, so no other machine
//! can reach it and Windows does not ask to open the firewall. Only `GET /callback` carrying this
//! sign-in's `state` ends it; anything else, such as a favicon request or a stale tab from an
//! earlier attempt, gets 404 and the listener keeps waiting. Dropping the waiting future closes the
//! socket and every connection it accepted, which is how cancel, timeout and shutdown release it.

use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use url::Url;

const CALLBACK_PATH: &str = "/callback";
/// Bounds the request head; a redirect is one short line plus a few headers.
const MAX_REQUEST_HEAD: usize = 8 * 1024;
/// How long one connection may take to send its request head. Browsers open idle speculative
/// connections; each is handled on its own task, so a silent one only holds itself.
const REQUEST_HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Shown before the code is exchanged, so it claims only that the approval arrived.
const RECEIVED_PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><title>Ora</title>\
<p>已收到授权，请回到 Ora。</p><p>Authorization received. Return to Ora.</p>";
const DENIED_PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><title>Ora</title>\
<p>登录没有完成，可以回到 Ora 重试。</p><p>Sign-in did not complete. Return to Ora to try again.</p>";

/// Why the redirect did not deliver a code.
#[derive(Debug, Error)]
pub(crate) enum LoopbackError {
    /// The redirect carried an error, or no code, for this sign-in.
    #[error("the browser returned no authorization code")]
    Denied,
    /// The listener itself failed.
    #[error("loopback listener failed")]
    Io(#[from] std::io::Error),
}

/// A bound loopback listener waiting for one redirect.
pub(crate) struct LoopbackListener {
    listener: TcpListener,
    redirect_uri: String,
}

/// What one accepted connection turned out to be.
enum Delivery {
    /// Not this sign-in's redirect; keep waiting.
    Ignored,
    Code(String),
    Denied,
}

impl LoopbackListener {
    /// Binds `127.0.0.1:0`; the redirect URI names the IP, never `localhost` (RFC 8252 §8.3).
    pub(crate) async fn bind() -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        Ok(Self {
            listener,
            redirect_uri: format!("http://127.0.0.1:{port}{CALLBACK_PATH}"),
        })
    }

    /// The exact redirect URI the server must send the browser back to.
    pub(crate) fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// Waits for the redirect carrying `state` and returns its code.
    pub(crate) async fn wait(self, state: String) -> Result<String, LoopbackError> {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted?;
                    let state = state.clone();
                    connections.spawn(async move { serve(stream, &state).await });
                }
                Some(joined) = connections.join_next() => {
                    match joined {
                        Ok(Delivery::Code(code)) => return Ok(code),
                        Ok(Delivery::Denied) => return Err(LoopbackError::Denied),
                        // A broken connection or panicked handler says nothing about the sign-in.
                        Ok(Delivery::Ignored) | Err(_) => {}
                    }
                }
            }
        }
    }
}

/// Reads one request head and answers it; only a matching redirect yields a delivery.
async fn serve(mut stream: TcpStream, state: &str) -> Delivery {
    let Ok(Some(target)) = tokio::time::timeout(REQUEST_HEAD_TIMEOUT, request_target(&mut stream))
        .await
        .unwrap_or(Ok(None))
    else {
        return Delivery::Ignored;
    };
    let delivery = classify(&target, state);
    let (status, body) = match &delivery {
        Delivery::Ignored => ("404 Not Found", ""),
        Delivery::Code(_) => ("200 OK", RECEIVED_PAGE),
        Delivery::Denied => ("200 OK", DENIED_PAGE),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // The outcome stands even if the browser has already gone; the page is a courtesy.
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
    delivery
}

/// Returns the request target of a `GET`, or `None` for anything that is not one.
async fn request_target(stream: &mut TcpStream) -> std::io::Result<Option<String>> {
    let mut head = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        if head.len() > MAX_REQUEST_HEAD {
            return Ok(None);
        }
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&buffer[..read]);
    }
    let text = String::from_utf8_lossy(&head);
    let mut parts = text.lines().next().unwrap_or_default().split(' ');
    Ok(match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) => Some(target.to_owned()),
        _ => None,
    })
}

/// Decides what a request target means for the sign-in waiting on `state`.
fn classify(target: &str, state: &str) -> Delivery {
    let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Delivery::Ignored;
    };
    if url.path() != CALLBACK_PATH {
        return Delivery::Ignored;
    }
    let value = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    // A redirect for another attempt must not end this one, whatever it carries.
    if value("state").as_deref() != Some(state) {
        return Delivery::Ignored;
    }
    match (value("code"), value("error")) {
        (Some(code), None) if !code.is_empty() => Delivery::Code(code),
        (Some(_), Some(_)) | (Some(_), None) | (None, _) => Delivery::Denied,
    }
}

#[cfg(test)]
mod tests {
    use super::{LoopbackError, LoopbackListener};
    use pretty_assertions::assert_eq;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    /// Sends one GET to the listener the way a browser would and returns the status line.
    async fn browser_get(redirect_uri: &str, query: &str) -> String {
        let url = url::Url::parse(redirect_uri).expect("redirect uri");
        let address = format!("127.0.0.1:{}", url.port().expect("port"));
        get(&address, &format!("{}?{query}", url.path())).await
    }

    /// Sends a raw GET for `target` and returns the response status line.
    async fn get(address: &str, target: &str) -> String {
        let mut stream = TcpStream::connect(address).await.expect("connect");
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
            .await
            .expect("send");
        let mut response = String::new();
        stream.read_to_string(&mut response).await.expect("read");
        response.lines().next().unwrap_or_default().to_owned()
    }

    /// Only the matching redirect ends the wait; strays get 404 and are ignored.
    #[tokio::test]
    async fn ignores_strays_and_returns_the_matching_code() {
        let listener = LoopbackListener::bind().await.expect("bind");
        let redirect = listener.redirect_uri().to_owned();
        assert!(redirect.starts_with("http://127.0.0.1:"));
        let waiting = tokio::spawn(listener.wait("good".to_owned()));
        let address = redirect
            .trim_start_matches("http://")
            .trim_end_matches("/callback")
            .to_owned();
        assert_eq!(
            get(&address, "/favicon.ico").await,
            "HTTP/1.1 404 Not Found"
        );
        assert_eq!(
            browser_get(&redirect, "code=stolen&state=other").await,
            "HTTP/1.1 404 Not Found"
        );
        assert_eq!(
            browser_get(&redirect, "code=c1&state=good").await,
            "HTTP/1.1 200 OK"
        );
        assert_eq!(waiting.await.expect("join").expect("code"), "c1");
        // The listener is closed once the wait is over.
        assert!(TcpStream::connect(&address).await.is_err());
    }

    /// An error redirect for this sign-in ends it as denied.
    #[tokio::test]
    async fn error_redirect_is_denied() {
        let listener = LoopbackListener::bind().await.expect("bind");
        let redirect = listener.redirect_uri().to_owned();
        let waiting = tokio::spawn(listener.wait("good".to_owned()));
        browser_get(&redirect, "error=access_denied&state=good").await;
        assert!(matches!(
            waiting.await.expect("join"),
            Err(LoopbackError::Denied)
        ));
    }
}
