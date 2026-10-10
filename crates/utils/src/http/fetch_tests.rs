#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use pretty_assertions::assert_eq;
use std::os::unix::fs::PermissionsExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One request as the stub server saw it.
#[derive(Debug, PartialEq, Eq)]
struct Received {
    request_line: String,
    proof: Option<String>,
}

/// Serves exactly one HTTP/1.1 request with a raw `response` and returns what it received.
async fn serve_once(response: Vec<u8>) -> (String, tokio::task::JoinHandle<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1024];
            let count = stream.read(&mut chunk).await.unwrap();
            assert!(count > 0, "client closed before headers");
            buffer.extend_from_slice(&chunk[..count]);
            if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                break end;
            }
        };
        let head = String::from_utf8(buffer[..header_end].to_vec()).unwrap();
        let mut lines = head.split("\r\n");
        let request_line = lines.next().unwrap().to_owned();
        let proof = lines.find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("x-amz-meta-proof")
                .then(|| value.trim().to_owned())
        });
        stream.write_all(&response).await.unwrap();
        Received {
            request_line,
            proof,
        }
    });
    (
        format!("http://{address}/bucket/key?X-Amz-Signature=secret"),
        handle,
    )
}

/// A complete response with `status` and `body`.
fn response(status: u16, extra: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\n{extra}connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// Bounds every test request.
fn options() -> FetchOptions {
    FetchOptions {
        connect_timeout: Duration::from_secs(/*secs*/ 5),
        total_timeout: Duration::from_secs(/*secs*/ 10),
    }
}

/// The signed headers every test request carries.
fn headers() -> BTreeMap<String, String> {
    [("x-amz-meta-proof".to_owned(), "signed".to_owned())].into()
}

/// Runs one fetch into `destination` with a 1 KiB limit.
async fn fetch(url: &str, destination: &Path) -> Result<FetchOutcome, FetchError> {
    ReqwestFetcher::new(ProxyConfig::default())
        .fetch(
            FileFetch {
                url,
                headers: &headers(),
                destination,
                max_bytes: 1024,
            },
            options(),
        )
        .await
}

/// A success body lands in a new owner-only file with its measured size and digest, and the
/// signed headers reach the server unchanged.
#[tokio::test]
async fn stores_a_success_body_owner_only_with_its_digest() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("object");
    let (url, server) = serve_once(response(200, "", b"bundle bytes")).await;
    let outcome = fetch(&url, &destination).await.unwrap();
    assert_eq!(
        outcome,
        FetchOutcome::Stored {
            bytes: 12,
            sha256: Sha256::digest(b"bundle bytes").into(),
        }
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"bundle bytes");
    assert_eq!(
        std::fs::metadata(&destination)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        server.await.unwrap(),
        Received {
            request_line: "GET /bucket/key?X-Amz-Signature=secret HTTP/1.1".into(),
            proof: Some("signed".into()),
        }
    );
}

/// Other statuses, redirects included, are reported without following them or writing anything.
#[tokio::test]
async fn reports_other_statuses_without_following_redirects() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("object");
    let (elsewhere, never) = serve_once(response(200, "", b"other origin")).await;
    let location = format!("location: {elsewhere}\r\n");
    for (status, extra) in [(302, location.as_str()), (403, ""), (503, "")] {
        let (url, _server) = serve_once(response(status, extra, b"")).await;
        assert_eq!(
            fetch(&url, &destination).await.unwrap(),
            FetchOutcome::Status(status)
        );
        assert!(!destination.exists());
    }
    never.abort();
}

/// A body over the limit, or a destination that already exists, is refused and leaves no file
/// behind; no error ever names the signed URL.
#[tokio::test]
async fn refuses_oversized_bodies_and_existing_destinations_without_the_url() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("object");
    let (url, _server) = serve_once(response(200, "", &[7_u8; 2048])).await;
    let error = fetch(&url, &destination).await.unwrap_err();
    assert!(
        matches!(error, FetchError::TooLarge { limit: 1024 }),
        "{error:?}"
    );
    assert!(!destination.exists());

    std::fs::write(&destination, b"kept").unwrap();
    let (url, _server) = serve_once(response(200, "", b"new")).await;
    let error = fetch(&url, &destination).await.unwrap_err();
    assert!(matches!(error, FetchError::Io { .. }), "{error:?}");
    assert_eq!(std::fs::read(&destination).unwrap(), b"kept");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed = format!(
        "http://{}/bucket/key?X-Amz-Signature=secret",
        listener.local_addr().unwrap()
    );
    drop(listener);
    let error = fetch(&closed, &directory.path().join("absent"))
        .await
        .unwrap_err();
    assert!(matches!(error, FetchError::Network(_)), "{error:?}");
    assert!(!error.to_string().contains("secret"), "{error}");
}
