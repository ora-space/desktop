//! Unit tests for the local downloader and its data types.

use futures::executor::block_on;
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use tempfile::TempDir;
use url::Url;

use super::error::DownloadError;
use super::local::LocalFileDownloader;
use super::types::{Checksum, DownloadOptions, DownloadRequest, DownloadSource, HttpDownload};

/// Computes the SHA-256 digest of `data` as raw bytes for building expectations.
fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Builds a request against a local path destination with the given source, checksum, and limit.
fn request(
    source: DownloadSource,
    destination: &Path,
    checksum: Option<Checksum>,
    max_bytes: Option<u64>,
) -> DownloadRequest {
    DownloadRequest {
        source,
        destination: destination.to_path_buf(),
        checksum,
        options: DownloadOptions {
            max_bytes,
            ..Default::default()
        },
        progress: None,
        cancel: None,
    }
}

/// Runs a download through the public trait API and returns its result.
fn download(request: DownloadRequest) -> Result<super::types::DownloadOutcome, DownloadError> {
    block_on(LocalFileDownloader.download(request))
}

/// Copies a local file to a destination and reports the exact byte count.
#[test]
fn copies_local_file_and_reports_bytes() {
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("artifact.orax");
    let destination = temp_dir.path().join("out").join("installed.orax");
    let payload = b"hello world".to_vec();
    fs::write(&source, &payload).unwrap();

    let outcome = download(request(
        DownloadSource::Local(source),
        &destination,
        None,
        None,
    ))
    .unwrap();

    assert_eq!(outcome.bytes, payload.len() as u64);
    assert_eq!(outcome.sha256, sha256_bytes(&payload));
    assert_eq!(fs::read(&destination).unwrap(), payload);
}

/// Object-key sources are rejected by the local downloader instead of being treated as paths.
#[test]
fn rejects_s3_object_key_sources() {
    let temp_dir = TempDir::new().unwrap();
    let destination = temp_dir.path().join("out.bin");
    let error = download(request(
        DownloadSource::S3 {
            key: "pkg.orax".to_owned(),
        },
        &destination,
        None,
        None,
    ))
    .unwrap_err();
    match error {
        DownloadError::InvalidSource(message) => {
            assert!(message.contains("S3"));
        }
        other => panic!("expected invalid source, got {other:?}"),
    }
}
#[test]
fn verifies_correct_sha256() {
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("artifact.bin");
    let destination = temp_dir.path().join("verify.bin");
    let payload = b"verified".to_vec();
    fs::write(&source, &payload).unwrap();

    download(request(
        DownloadSource::Local(source),
        &destination,
        Some(Checksum::sha256(sha256_bytes(&payload))),
        None,
    ))
    .unwrap();

    assert!(destination.exists());
}

/// Rejects a wrong digest, reports the mismatch, and leaves no destination or temporary file.
#[test]
fn rejects_wrong_checksum_and_leaves_no_output() {
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("artifact.bin");
    let destination = temp_dir.path().join("bad.bin");
    fs::write(&source, b"actual").unwrap();

    let error = download(request(
        DownloadSource::Local(source),
        &destination,
        Some(Checksum::sha256([0_u8; 32])),
        None,
    ))
    .unwrap_err();

    match error {
        DownloadError::ChecksumMismatch { actual, .. } => {
            assert_eq!(actual, sha256_bytes(b"actual"));
        }
        other => panic!("expected checksum mismatch, got {other:?}"),
    }
    assert!(!destination.exists());
    assert!(!temp_dir.path().join("bad.bin.tmp").exists());
}

/// Aborts at the byte limit, reports it, and cleans up both destination and temporary file.
#[test]
fn enforces_max_bytes_limit() {
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("large.bin");
    let destination = temp_dir.path().join("limited.bin");
    fs::write(&source, vec![0_u8; 10]).unwrap();

    let error = download(request(
        DownloadSource::Local(source),
        &destination,
        None,
        Some(5),
    ))
    .unwrap_err();

    match error {
        DownloadError::TooLarge { limit, .. } => assert_eq!(limit, 5),
        other => panic!("expected too-large error, got {other:?}"),
    }
    assert!(!destination.exists());
    assert!(!temp_dir.path().join("limited.bin.tmp").exists());
}

/// Reads the source through a `file://` URL.
#[test]
fn downloads_from_file_url() {
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("remote.bin");
    let destination = temp_dir.path().join("copied.bin");
    let payload = b"via file url".to_vec();
    fs::write(&source, &payload).unwrap();
    let url = Url::from_file_path(&source).unwrap();

    let outcome = download(request(DownloadSource::Url(url), &destination, None, None)).unwrap();

    assert_eq!(outcome.bytes, payload.len() as u64);
    assert_eq!(fs::read(&destination).unwrap(), payload);
}

/// The local downloader refuses remote and unsupported schemes.
#[test]
fn rejects_remote_url_source() {
    let temp_dir = TempDir::new().unwrap();
    let destination = temp_dir.path().join("out.bin");

    let http_error = download(request(
        DownloadSource::Url(Url::parse("https://example.com/pkg.orax").unwrap()),
        &destination,
        None,
        None,
    ))
    .unwrap_err();
    match http_error {
        DownloadError::InvalidSource(message) => assert!(message.contains("https")),
        other => panic!("expected invalid source, got {other:?}"),
    }

    let ftp_error = download(request(
        DownloadSource::Url(Url::parse("ftp://example.com/pkg.orax").unwrap()),
        &destination,
        None,
        None,
    ))
    .unwrap_err();
    match ftp_error {
        DownloadError::InvalidSource(_) => {}
        other => panic!("expected invalid source, got {other:?}"),
    }
}

/// A successful download atomically replaces a pre-existing destination.
#[test]
fn atomically_replaces_existing_destination() {
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("new.bin");
    let destination = temp_dir.path().join("existing.bin");
    fs::write(&source, b"new content").unwrap();
    fs::write(&destination, b"old content").unwrap();

    download(request(
        DownloadSource::Local(source),
        &destination,
        None,
        None,
    ))
    .unwrap();

    assert_eq!(fs::read(&destination).unwrap(), b"new content");
    assert!(!temp_dir.path().join("existing.bin.tmp").exists());
}

/// A failed download leaves a pre-existing destination untouched.
#[test]
fn preserves_destination_on_failure() {
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("new.bin");
    let destination = temp_dir.path().join("existing.bin");
    fs::write(&source, b"new content").unwrap();
    fs::write(&destination, b"old content").unwrap();

    let _ = download(request(
        DownloadSource::Local(source),
        &destination,
        Some(Checksum::sha256([0_u8; 32])),
        None,
    ));

    assert_eq!(fs::read(&destination).unwrap(), b"old content");
}

/// Reports an unreadable source path as an I/O error rather than panicking.
#[test]
fn reports_missing_source_as_io_error() {
    let temp_dir = TempDir::new().unwrap();
    let destination = temp_dir.path().join("out.bin");
    let missing = temp_dir.path().join("does-not-exist.bin");

    let error = download(request(
        DownloadSource::Local(missing),
        &destination,
        None,
        None,
    ))
    .unwrap_err();

    match error {
        DownloadError::Io { .. } => {}
        other => panic!("expected io error, got {other:?}"),
    }
}

/// Default options leave the byte bound unset.
#[test]
fn default_options_have_no_byte_limit() {
    assert_eq!(DownloadOptions::default().max_bytes, None);
}

/// Integration coverage for the network backend, exercised against a loopback server.
#[cfg(feature = "http-reqwest")]
mod reqwest_integration {
    use crate::http::{
        CancelToken, Checksum, DownloadError, DownloadOptions, DownloadRequest, DownloadSource,
        HttpDownload, ReqwestDownloader,
    };
    use pretty_assertions::assert_eq;
    use sha2::{Digest, Sha256};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;
    use url::Url;

    fn sha256_bytes(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    /// Accepts one loopback connection, answers it with a 200 response carrying `body`, and returns
    /// the URL the caller should request.
    async fn serve_once(body: &'static [u8]) -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 4096];
            let _ = socket.read(&mut buffer).await.unwrap();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
            let _ = socket.flush().await;
        });
        Url::parse(&format!("http://127.0.0.1:{port}/pkg.orax")).unwrap()
    }

    /// Serves one HTTPS response from a certificate signed by a private test CA.
    async fn serve_https_once(
        body: &'static [u8],
    ) -> (Url, rustls::pki_types::CertificateDer<'static>) {
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        let ca = rcgen::CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();
        let server_key = rcgen::KeyPair::generate().unwrap();
        let mut server_params =
            rcgen::CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
        server_params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        server_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let server_cert = server_params.signed_by(&server_key, &ca).unwrap();
        let private_key = rustls::pki_types::PrivatePkcs8KeyDer::from(server_key.serialize_der());
        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![server_cert.der().clone()], private_key.into())
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(server_config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let Ok(mut socket) = acceptor.accept(socket).await else {
                return;
            };
            let mut buffer = [0_u8; 4096];
            let _bytes_read = socket.read(&mut buffer).await.unwrap();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
            let _flush_result = socket.flush().await;
        });
        (
            Url::parse(&format!("https://localhost:{port}/pkg.orax")).unwrap(),
            ca.der().clone(),
        )
    }

    /// Downloads through a private CA when that CA is explicitly trusted by the test client.
    #[tokio::test]
    async fn downloads_from_explicitly_trusted_private_ca() {
        let payload: &'static [u8] = b"enterprise release package";
        let (url, ca_root) = serve_https_once(payload).await;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let destination = temp_dir.path().join("pkg.orax");
        let downloader = ReqwestDownloader::new(Default::default()).with_extra_tls_root(ca_root);

        let outcome = downloader
            .download(DownloadRequest {
                source: DownloadSource::Url(url),
                destination: destination.clone(),
                checksum: Some(Checksum::sha256(sha256_bytes(payload))),
                options: DownloadOptions {
                    max_retries: 0,
                    ..DownloadOptions::default()
                },
                progress: None,
                cancel: None,
            })
            .await
            .unwrap();

        assert_eq!(outcome.bytes, payload.len() as u64);
        assert_eq!(std::fs::read(&destination).unwrap(), payload);
    }

    /// Rejects the same private CA when it has not been added to platform trust for the client.
    #[tokio::test]
    async fn rejects_untrusted_private_ca() {
        let (url, _ca_root) = serve_https_once(b"untrusted release package").await;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let destination = temp_dir.path().join("pkg.orax");
        let downloader = ReqwestDownloader::new(Default::default());

        let error = downloader
            .download(DownloadRequest {
                source: DownloadSource::Url(url),
                destination,
                checksum: None,
                options: DownloadOptions {
                    max_retries: 0,
                    ..DownloadOptions::default()
                },
                progress: None,
                cancel: None,
            })
            .await
            .unwrap_err();

        let message = match error {
            DownloadError::Network { source, .. } => source.to_string(),
            other => panic!("expected network error, got {other:?}"),
        };
        assert!(
            message.contains("invalid peer certificate"),
            "expected certificate rejection, got: {message}"
        );
    }

    /// Streams a payload from a loopback server into a destination and verifies bytes + checksum.
    #[tokio::test]
    async fn downloads_from_local_http_server() {
        let payload: &'static [u8] = b"hello network world";
        let url = serve_once(payload).await;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let destination = temp_dir.path().join("pkg.orax");
        let downloader = ReqwestDownloader::new(Default::default());

        let outcome = downloader
            .download(DownloadRequest {
                source: DownloadSource::Url(url),
                destination: destination.clone(),
                checksum: Some(Checksum::sha256(sha256_bytes(payload))),
                options: DownloadOptions::default(),
                progress: None,
                cancel: None,
            })
            .await
            .unwrap();

        assert_eq!(outcome.bytes, payload.len() as u64);
        assert_eq!(std::fs::read(&destination).unwrap(), payload);
    }

    /// A connectivity probe reports the HTTP status without writing a destination file.
    #[tokio::test]
    async fn probe_reports_http_status_from_local_server() {
        let url = serve_once(b"ok").await;
        let status = ReqwestDownloader::new(Default::default())
            .probe(url, std::time::Duration::from_secs(2))
            .await
            .unwrap();

        assert_eq!(status, 200);
    }

    /// A refused loopback port is reported as a network error rather than a successful status.
    #[tokio::test]
    async fn probe_reports_a_refused_connection() {
        let url = Url::parse("http://127.0.0.1:1/").unwrap();
        let error = ReqwestDownloader::new(Default::default())
            .probe(url, std::time::Duration::from_secs(2))
            .await
            .unwrap_err();

        assert!(matches!(error, DownloadError::Network { .. }));
    }

    /// A response larger than the byte limit is rejected and leaves no destination file.
    #[tokio::test]
    async fn rejects_response_over_byte_limit() {
        let payload: &'static [u8] = b"1234567890";
        let url = serve_once(payload).await;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let destination = temp_dir.path().join("pkg.orax");
        let downloader = ReqwestDownloader::new(Default::default());

        let error = downloader
            .download(DownloadRequest {
                source: DownloadSource::Url(url),
                destination: destination.clone(),
                checksum: None,
                options: DownloadOptions {
                    max_bytes: Some(5),
                    ..Default::default()
                },
                progress: None,
                cancel: None,
            })
            .await
            .unwrap_err();

        match error {
            DownloadError::TooLarge { limit, .. } => assert_eq!(limit, 5),
            other => panic!("expected too-large error, got {other:?}"),
        }
        assert!(!destination.exists());
    }

    /// A pre-cancelled token aborts the transfer before any bytes are written.
    #[tokio::test]
    async fn abort_when_cancelled() {
        let payload: &'static [u8] = b"payload to abandon";
        let url = serve_once(payload).await;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let destination = temp_dir.path().join("pkg.orax");
        let token = CancelToken::new();
        token.cancel();
        let downloader = ReqwestDownloader::new(Default::default());

        let error = downloader
            .download(DownloadRequest {
                source: DownloadSource::Url(url),
                destination: destination.clone(),
                checksum: None,
                options: DownloadOptions::default(),
                progress: None,
                cancel: Some(token),
            })
            .await
            .unwrap_err();

        match error {
            DownloadError::Cancelled => {}
            other => panic!("expected cancellation, got {other:?}"),
        }
        assert!(!destination.exists());
    }

    /// A failed connection surfaces the full reqwest cause chain instead of only the outermost
    /// message, so the real TLS/connect reason stays visible.
    ///
    /// We point at a loopback port that refuses connections (`127.0.0.1:1`), which forces a
    /// connect-time failure whose error carries an inner cause (the underlying connect error).
    #[tokio::test]
    async fn network_error_preserves_cause_chain() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let destination = temp_dir.path().join("pkg.orax");
        let downloader = ReqwestDownloader::new(Default::default());
        // Port 1 is reserved and refuses TCP connections, guaranteeing a connect-time failure.
        let url = Url::parse("http://127.0.0.1:1/pkg.orax").unwrap();

        let error = downloader
            .download(DownloadRequest {
                source: DownloadSource::Url(url),
                destination: destination.clone(),
                checksum: None,
                options: DownloadOptions::default(),
                progress: None,
                cancel: None,
            })
            .await
            .unwrap_err();

        let message = match error {
            DownloadError::Network { source, .. } => source.to_string(),
            other => panic!("expected network error, got {other:?}"),
        };
        // The flattened chain must keep the outermost message plus at least one inner cause,
        // proving the " <- " joining happened instead of only `error.to_string()`.
        assert!(
            message.contains(" <- "),
            "expected a cause chain joined by ' <- ', got: {message}"
        );
    }

    /// An object-key download signs the GET and still verifies the payload checksum.
    #[tokio::test]
    async fn downloads_s3_object_key_with_signed_headers() {
        use crate::http::{S3AwareDownloader, S3Config};
        use std::sync::{Arc, Mutex};

        let payload: &'static [u8] = b"s3 object payload";
        let captured: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let (url, ca_root) = serve_https_once_capturing(payload, Arc::clone(&captured)).await;
        let port = url.port().unwrap();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let destination = temp_dir.path().join("pkg.orax");
        let config = S3Config::new(
            format!("localhost:{port}"),
            "bucket",
            "dgg",
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        );
        let downloader = S3AwareDownloader::new(
            ReqwestDownloader::new(Default::default()).with_extra_tls_root(ca_root),
            Some(config),
        );

        let outcome = downloader
            .download(DownloadRequest {
                source: DownloadSource::S3 {
                    key: "pkg.orax".to_owned(),
                },
                destination: destination.clone(),
                checksum: Some(Checksum::sha256(sha256_bytes(payload))),
                options: DownloadOptions {
                    max_retries: 0,
                    ..DownloadOptions::default()
                },
                progress: None,
                cancel: None,
            })
            .await
            .unwrap();

        assert_eq!(outcome.bytes, payload.len() as u64);
        assert_eq!(std::fs::read(&destination).unwrap(), payload);
        let request = captured.lock().unwrap().clone();
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: aws4-hmac-sha256"),
            "expected a SigV4 authorization header, got: {request}"
        );
        assert!(
            request.to_ascii_lowercase().contains("x-amz-date:"),
            "expected x-amz-date, got: {request}"
        );
    }

    /// A path-style HTTPS URL that targets the configured bucket is signed like an object key.
    #[tokio::test]
    async fn downloads_path_style_bucket_https_url_with_signed_headers() {
        use crate::http::{S3AwareDownloader, S3Config, path_style_object_url};
        use std::sync::{Arc, Mutex};

        let payload: &'static [u8] = b"path style s3 payload";
        let captured: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let (listener_url, ca_root) =
            serve_https_once_capturing(payload, Arc::clone(&captured)).await;
        let port = listener_url.port().unwrap();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let destination = temp_dir.path().join("pkg.orax");
        let endpoint = format!("localhost:{port}");
        let config = S3Config::new(
            endpoint.clone(),
            "plugin-artifacts",
            "dgg",
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        );
        let source_url = path_style_object_url(
            &endpoint,
            "plugin-artifacts",
            "agent/example.codeagent-v0.5.1.orax",
        )
        .unwrap();
        let downloader = S3AwareDownloader::new(
            ReqwestDownloader::new(Default::default()).with_extra_tls_root(ca_root),
            Some(config),
        );

        let outcome = downloader
            .download(DownloadRequest {
                source: DownloadSource::Url(source_url),
                destination: destination.clone(),
                checksum: Some(Checksum::sha256(sha256_bytes(payload))),
                options: DownloadOptions {
                    max_retries: 0,
                    ..DownloadOptions::default()
                },
                progress: None,
                cancel: None,
            })
            .await
            .unwrap();

        assert_eq!(outcome.bytes, payload.len() as u64);
        assert_eq!(std::fs::read(&destination).unwrap(), payload);
        let request = captured.lock().unwrap().clone();
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: aws4-hmac-sha256"),
            "expected a SigV4 authorization header, got: {request}"
        );
        assert!(
            request.contains("plugin-artifacts/agent/example.codeagent-v0.5.1.orax"),
            "expected the nested object path in the request line, got: {request}"
        );
    }

    /// S3 mode rejects a foreign HTTPS locator instead of silently bypassing SigV4.
    #[tokio::test]
    async fn rejects_foreign_https_url_in_s3_mode() {
        use crate::http::{S3AwareDownloader, S3Config};

        let temp_dir = tempfile::TempDir::new().unwrap();
        let downloader = S3AwareDownloader::new(
            ReqwestDownloader::new(Default::default()),
            Some(S3Config::new(
                "s3.example.com",
                "plugin-artifacts",
                "region-1",
                "access",
                "secret",
            )),
        );
        let error = downloader
            .download(DownloadRequest {
                source: DownloadSource::Url(
                    Url::parse("https://downloads.example.com/plugin.orax").unwrap(),
                ),
                destination: temp_dir.path().join("plugin.orax"),
                checksum: None,
                options: DownloadOptions::default(),
                progress: None,
                cancel: None,
            })
            .await
            .unwrap_err();

        assert!(matches!(error, DownloadError::InvalidSource(_)));
    }

    /// Object-key sources fail closed when no S3 endpoint is configured.
    #[tokio::test]
    async fn rejects_s3_object_key_without_config() {
        use crate::http::S3AwareDownloader;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let destination = temp_dir.path().join("pkg.orax");
        let downloader = S3AwareDownloader::new(ReqwestDownloader::new(Default::default()), None);
        let error = downloader
            .download(DownloadRequest {
                source: DownloadSource::S3 {
                    key: "pkg.orax".to_owned(),
                },
                destination,
                checksum: None,
                options: DownloadOptions {
                    max_retries: 0,
                    ..DownloadOptions::default()
                },
                progress: None,
                cancel: None,
            })
            .await
            .unwrap_err();
        match error {
            DownloadError::InvalidSource(message) => {
                assert!(message.contains("not configured"));
            }
            other => panic!("expected invalid source, got {other:?}"),
        }
    }

    /// Serves one HTTPS response and records the raw request for header assertions.
    async fn serve_https_once_capturing(
        body: &'static [u8],
        captured: std::sync::Arc<std::sync::Mutex<String>>,
    ) -> (Url, rustls::pki_types::CertificateDer<'static>) {
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        let ca = rcgen::CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();
        let server_key = rcgen::KeyPair::generate().unwrap();
        let mut server_params =
            rcgen::CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
        server_params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        server_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let server_cert = server_params.signed_by(&server_key, &ca).unwrap();
        let private_key = rustls::pki_types::PrivatePkcs8KeyDer::from(server_key.serialize_der());
        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![server_cert.der().clone()], private_key.into())
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(server_config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let Ok(mut socket) = acceptor.accept(socket).await else {
                return;
            };
            let mut buffer = [0_u8; 8192];
            let bytes_read = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..bytes_read]).into_owned();
            *captured.lock().unwrap() = request;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
            let _flush_result = socket.flush().await;
        });
        (
            Url::parse(&format!("https://localhost:{port}/pkg.orax")).unwrap(),
            ca.der().clone(),
        )
    }
}
