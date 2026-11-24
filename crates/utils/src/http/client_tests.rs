//! The credential client must not pick up an environment proxy for a host the configuration sends
//! direct. The proxy variables are set only in a child copy of this test binary, so the process
//! environment of the other tests is never touched.

use std::process::Command;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

use super::credential_client;
use crate::http::ProxyConfig;

const CHILD: &str = "http::client::tests::loopback_request_goes_direct";

/// Runs the direct-connection check in a child whose environment points every scheme at a proxy
/// that does not exist and does not exempt loopback.
#[test]
fn environment_proxy_is_not_used_for_loopback() {
    let output = Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", CHILD, "--ignored", "--test-threads=1"])
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("http_proxy", "http://127.0.0.1:9")
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .output()
        .expect("run child test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Child half: a request to a loopback server must reach it directly.
#[tokio::test]
#[ignore = "run by environment_proxy_is_not_used_for_loopback with proxy variables set"]
async fn loopback_request_goes_direct() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buffer = [0_u8; 1024];
        let _ = stream.read(&mut buffer).await;
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n")
            .await
            .expect("respond");
    });
    let url = Url::parse(&format!("http://{address}/me")).expect("url");
    let client = credential_client(
        &ProxyConfig::default(),
        &url,
        Duration::from_secs(5),
        Duration::from_secs(5),
    )
    .expect("client");
    let response = client.get(url).send().await.expect("direct request");
    assert_eq!(response.status().as_u16(), 204);
    server.await.expect("server");
}
