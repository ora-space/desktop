#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use pretty_assertions::assert_eq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One request as the stub server saw it.
#[derive(Debug, PartialEq, Eq)]
struct Received {
    request_line: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

/// Serves exactly one HTTP/1.1 request with `status` and returns what it received.
async fn serve_once(
    status: u16,
    extra: &'static str,
) -> (String, tokio::task::JoinHandle<Received>) {
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
        let headers: BTreeMap<String, String> = lines
            .map(|line| {
                let (name, value) = line.split_once(':').unwrap();
                (name.to_ascii_lowercase(), value.trim().to_owned())
            })
            .collect();
        let length: usize = headers["content-length"].parse().unwrap();
        let mut body = buffer[header_end + 4..].to_vec();
        while body.len() < length {
            let mut chunk = [0_u8; 1024];
            let count = stream.read(&mut chunk).await.unwrap();
            body.extend_from_slice(&chunk[..count]);
        }
        stream
            .write_all(
                format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: 0\r\n{extra}connection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        Received {
            request_line,
            headers,
            body,
        }
    });
    (
        format!("http://{address}/bucket/key?X-Amz-Signature=secret"),
        handle,
    )
}

/// Bounds every test request.
fn options() -> UploadOptions {
    UploadOptions {
        connect_timeout: Duration::from_secs(/*secs*/ 5),
        total_timeout: Duration::from_secs(/*secs*/ 10),
    }
}

/// Signed headers, including conditional-create and checksum headers, reach the server unchanged
/// beside the exact body and its length.
#[tokio::test]
async fn sends_signed_headers_verbatim_with_the_file_body() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("object");
    std::fs::write(&file, b"frozen bytes").unwrap();
    let (url, server) = serve_once(/*status*/ 201, "").await;
    let headers: BTreeMap<String, String> = [
        ("If-None-Match".to_owned(), "*".to_owned()),
        (
            "x-amz-checksum-sha256".to_owned(),
            "c2lnbmVkLWRpZ2VzdA==".to_owned(),
        ),
    ]
    .into();
    let status = ReqwestUploader::new(ProxyConfig::default())
        .send(
            FileUpload {
                method: "PUT",
                url: &url,
                headers: &headers,
                file: &file,
            },
            options(),
        )
        .await
        .unwrap();
    assert_eq!(status, 201);
    let mut received = server.await.unwrap();
    received.headers.retain(|name, _| {
        matches!(
            name.as_str(),
            "if-none-match" | "x-amz-checksum-sha256" | "content-length" | "transfer-encoding"
        )
    });
    assert_eq!(
        received,
        Received {
            request_line: "PUT /bucket/key?X-Amz-Signature=secret HTTP/1.1".into(),
            headers: [
                ("content-length".to_owned(), "12".to_owned()),
                ("if-none-match".to_owned(), "*".to_owned()),
                (
                    "x-amz-checksum-sha256".to_owned(),
                    "c2lnbmVkLWRpZ2VzdA==".to_owned()
                ),
            ]
            .into(),
            body: b"frozen bytes".to_vec(),
        }
    );
}

/// A redirect is reported as its own status instead of re-sending signed bytes elsewhere.
#[tokio::test]
async fn reports_redirects_without_following_them() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("object");
    std::fs::write(&file, b"x").unwrap();
    let (url, server) = serve_once(
        /*status*/ 307,
        "location: http://127.0.0.1:9/elsewhere\r\n",
    )
    .await;
    let status = ReqwestUploader::new(ProxyConfig::default())
        .send(
            FileUpload {
                method: "PUT",
                url: &url,
                headers: &BTreeMap::new(),
                file: &file,
            },
            options(),
        )
        .await
        .unwrap();
    assert_eq!(status, 307);
    server.await.unwrap();
}

/// A connection failure never echoes the presigned URL or its signature.
#[tokio::test]
async fn network_errors_do_not_expose_the_url() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("object");
    std::fs::write(&file, b"x").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let error = ReqwestUploader::new(ProxyConfig::default())
        .send(
            FileUpload {
                method: "PUT",
                url: &format!("http://{address}/key?X-Amz-Signature=secret"),
                headers: &BTreeMap::new(),
                file: &file,
            },
            options(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, UploadError::Network(_)));
    assert!(!error.to_string().contains("secret"));
}
