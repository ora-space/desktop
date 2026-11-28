use super::*;
use ora_application::{NetworkProxyMode, NetworkProxyPreferences, NetworkProxySettings};
use ora_backend::Settings;
use rustls::pki_types::PrivatePkcs8KeyDer;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// Replies to one request and returns the request line observed on the wire.
async fn respond(stream: &mut (impl AsyncRead + AsyncWrite + Unpin), response: &str) -> String {
    let mut reader = BufReader::new(&mut *stream);
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).await.unwrap() == 0 || header == "\r\n" {
            break;
        }
    }
    stream.write_all(response.as_bytes()).await.unwrap();
    line
}

/// Hosts a single HTTP redirect or final response on the loopback interface.
async fn serve_http(response: String) -> (Url, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        respond(&mut socket, &response).await
    });
    (url, handle)
}

/// Supplies a private CA and certificates valid for the HTTPS proxy and tunneled manifest host.
fn tls_fixture() -> (TlsAcceptor, updater_http::Certificate) {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::default();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca = rcgen::CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();
    let server_key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec![
        "localhost".to_owned(),
        "manifest.example.test".to_owned(),
    ])
    .unwrap()
    .signed_by(&server_key, &ca)
    .unwrap();
    let server = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.der().clone()],
        PrivatePkcs8KeyDer::from(server_key.serialize_der()).into(),
    )
    .unwrap();
    (
        TlsAcceptor::from(Arc::new(server)),
        updater_http::Certificate::from_der(ca.der()).unwrap(),
    )
}

/// Accepts an HTTP CONNECT, then serves the actual HTTPS response inside that tunnel.
async fn serve_connect(
    mut proxy: impl AsyncRead + AsyncWrite + Unpin,
    tls: TlsAcceptor,
) -> (String, String) {
    let connect = respond(&mut proxy, "HTTP/1.1 200 Connection Established\r\n\r\n").await;
    let mut origin = tls.accept(proxy).await.unwrap();
    let request = respond(&mut origin, OK).await;
    (connect, request)
}

/// Verifies actual transport parity for bare hosts, HTTP proxies and TLS-wrapped HTTPS proxies.
#[tokio::test]
async fn manual_address_formats_work_for_the_probe_and_updater_transport() {
    let (tls, ca) = tls_fixture();
    let data = tempfile::tempdir().unwrap();
    let backend =
        ora_backend::Backend::open(crate::tests::test_backend_paths(data.path())).unwrap();
    let settings = backend.settings();
    let manifest: Url = "https://manifest.example.test/latest.json".parse().unwrap();
    for host in ["localhost", "http://localhost", "https://localhost"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let preferences = NetworkProxyPreferences {
            mode: NetworkProxyMode::Manual,
            manual: Some(NetworkProxySettings {
                host: host.to_owned(),
                port: listener.local_addr().unwrap().port(),
                username: None,
                password: None,
            }),
        };
        let server_tls = tls.clone();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (socket, _) = listener.accept().await.unwrap();
                let (connect, request) = if host.starts_with("https://") {
                    let socket = server_tls.accept(socket).await.unwrap();
                    serve_connect(socket, server_tls.clone()).await
                } else {
                    serve_connect(socket, server_tls.clone()).await
                };
                assert!(
                    connect.starts_with("CONNECT manifest.example.test:443 "),
                    "{connect}"
                );
                assert!(request.starts_with("GET /latest.json "), "{request}");
            }
        });

        assert_eq!(
            check(
                updater_http::Client::builder().tls_certs_only([ca.clone()]),
                manifest.as_str(),
                || Settings::network_proxy_policy_for(&preferences),
            )
            .await,
            CheckProxySettingsResponse::Reachable {
                status: 200,
                route: Some(ProxyRoute::Manual),
            },
            "{host}"
        );
        let mut expected = preferences.clone();
        if host == "http://localhost" {
            expected.manual.as_mut().unwrap().host = "localhost".to_owned();
        }
        assert_eq!(
            settings
                .set_network_proxy_preferences(preferences)
                .await
                .unwrap(),
            expected
        );
        let saved_policy = settings.network_proxy_policy().await.unwrap();
        let response = configure_client(
            updater_http::Client::builder().tls_certs_only([ca.clone()]),
            &saved_policy,
        )
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
        .get(manifest.clone())
        .send()
        .await
        .unwrap();
        assert_eq!(response.status(), 200, "{host}");
        server.await.unwrap();
    }
}

/// Follows the configured proxy hop and then reports status and route at a direct destination.
#[tokio::test]
async fn reports_the_final_status_and_route_after_a_redirect() {
    for (status_line, expected) in [
        (
            "200 OK",
            CheckProxySettingsResponse::Reachable {
                status: 200,
                route: Some(ProxyRoute::Direct),
            },
        ),
        (
            "204 No Content",
            CheckProxySettingsResponse::Reachable {
                status: 204,
                route: Some(ProxyRoute::Direct),
            },
        ),
        (
            "403 Forbidden",
            CheckProxySettingsResponse::HttpError { status: 403 },
        ),
        (
            "404 Not Found",
            CheckProxySettingsResponse::HttpError { status: 404 },
        ),
    ] {
        let (destination, final_request) = serve_http(format!(
            "HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        ))
        .await;
        let (proxy, initial_request) = serve_http(format!(
            "HTTP/1.1 301 Moved Permanently\r\nLocation: {destination}final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )).await;
        let preferences = NetworkProxyPreferences {
            mode: NetworkProxyMode::Manual,
            manual: Some(NetworkProxySettings {
                host: "127.0.0.1".to_owned(),
                port: proxy.port().unwrap(),
                username: None,
                password: None,
            }),
        };
        assert_eq!(
            check(
                updater_http::Client::builder(),
                "http://origin.example/first",
                || { Settings::network_proxy_policy_for(&preferences) }
            )
            .await,
            expected
        );
        assert!(
            initial_request
                .await
                .unwrap()
                .starts_with("GET http://origin.example/first ")
        );
        assert!(final_request.await.unwrap().starts_with("GET /final "));
    }
}

/// A response without a redirect target cannot prove the destination was reached.
#[tokio::test]
async fn reports_a_redirect_without_location_as_an_http_error() {
    let (url, server) = serve_http(
        "HTTP/1.1 301 Moved Permanently\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_owned(),
    )
    .await;
    let preferences = NetworkProxyPreferences {
        mode: NetworkProxyMode::Direct,
        manual: None,
    };
    assert_eq!(
        check(updater_http::Client::builder(), url.as_str(), || {
            Settings::network_proxy_policy_for(&preferences)
        })
        .await,
        CheckProxySettingsResponse::HttpError { status: 301 }
    );
    server.await.unwrap();
}

/// The settings command reports both plain proxy and CONNECT challenges as authentication errors.
#[tokio::test]
async fn check_reports_proxy_authentication_failures() {
    for (version, url, request) in [
        (
            "HTTP/1.1",
            "http://origin.example/probe",
            "GET http://origin.example/probe ",
        ),
        (
            "HTTP/1.0",
            "https://origin.example/probe",
            "CONNECT origin.example:443 ",
        ),
        (
            "HTTP/1.1",
            "https://origin.example/probe",
            "CONNECT origin.example:443 ",
        ),
    ] {
        let (proxy, server) = serve_http(format!(
            "{version} 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=ora\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )).await;
        let preferences = NetworkProxyPreferences {
            mode: NetworkProxyMode::Manual,
            manual: Some(NetworkProxySettings {
                host: "127.0.0.1".to_owned(),
                port: proxy.port().unwrap(),
                username: None,
                password: None,
            }),
        };
        assert_eq!(
            check(updater_http::Client::builder(), url, || {
                Settings::network_proxy_policy_for(&preferences)
            })
            .await,
            CheckProxySettingsResponse::Unreachable {
                message: HostProxyError::Authentication.to_string()
            },
        );
        assert!(server.await.unwrap().starts_with(request));
    }
}

/// A settings check stops before sending to an origin whose route cannot be determined.
#[tokio::test]
async fn check_blocks_initial_and_redirected_targets_when_system_read_fails() {
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = direct.local_addr().unwrap();
    let host = "origin.example.test";
    let target = format!("http://{host}:{}/probe", address.port());
    let (initial, server) = serve_http(format!(
        "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    ))
    .await;
    for url in [target.as_str(), initial.as_str()] {
        let result = check(
            updater_http::Client::builder().resolve(host, address),
            url,
            || Ok(HostProxy::unavailable_system(HostProxyError::SystemRead)),
        )
        .await;
        assert!(matches!(
            result,
            CheckProxySettingsResponse::Unreachable { message }
                if message.contains("could not read the system proxy settings")
        ));
    }
    assert!(server.await.unwrap().starts_with("GET / "));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), direct.accept())
            .await
            .is_err(),
        "the origin must not receive a direct connection",
    );
}
