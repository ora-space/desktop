#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]
//! Real archive installation and lease isolation through the production installer API.
use ora_node::{PluginCatalog, PluginInstaller};
use ora_node_protocol::*;
use ora_utils::http::{
    DownloadError, DownloadOptions, DownloadOutcome, DownloadRequest, DownloadSource, HttpDownload,
    LocalFileDownloader,
};
use pretty_assertions::assert_eq;
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Observes the request reaching the real archive/checksum installer rather than its config builder.
struct PolicyDownload {
    archive: PathBuf,
    observed: Arc<Mutex<Vec<DownloadOptions>>>,
}
impl HttpDownload for PolicyDownload {
    /// Keeps byte/checksum verification while capturing the composed download policy.
    async fn download(
        &self,
        mut request: DownloadRequest,
    ) -> Result<DownloadOutcome, DownloadError> {
        self.observed.lock().unwrap().push(request.options);
        request.source = DownloadSource::Local(self.archive.clone());
        LocalFileDownloader.download(request).await
    }
}

/// Uses the production deployment parser without opening a database or starting native processes.
fn deployment_json() -> serde_json::Value {
    serde_json::json!({
        "node": {"home_directory": "/tmp/node", "identity": "Discover", "repositories": []},
        "process": {
            "host_directory": "/tmp/host", "expected_uid": 1000, "git_program": "/usr/bin/git",
            "environment": {}, "command_timeout_ms": 1000, "cleanup_timeout_ms": 1000,
            "shutdown_grace_ms": 100
        },
        "recovery_interval_ms": 50,
        "timezone": "Asia/Shanghai"
    })
}

/// Missing deployment fields preserve defaults; malformed or out-of-range values fail at parsing.
#[test]
fn deployment_plugin_timing_defaults_and_rejects_invalid_values() {
    let omitted: ora_node::ServiceConfig = serde_json::from_value(deployment_json()).unwrap();
    let mut empty = deployment_json();
    empty["plugins"] = serde_json::json!({});
    let empty: ora_node::ServiceConfig = serde_json::from_value(empty).unwrap();
    assert_eq!(
        (omitted.plugins, empty.plugins),
        (Default::default(), Default::default())
    );
    assert_eq!(
        serde_json::to_value(omitted.plugins).unwrap(),
        serde_json::json!({"download_timeout_seconds": 60})
    );
    for invalid in [
        serde_json::json!(0),
        serde_json::json!(9),
        serde_json::json!(1201),
        serde_json::json!(u64::MAX),
        serde_json::json!(-1),
        serde_json::json!(1.5),
        serde_json::json!("600"),
        serde_json::Value::Null,
    ] {
        let mut value = deployment_json();
        value["plugins"] = serde_json::json!({"download_timeout_seconds": invalid});
        assert!(serde_json::from_value::<ora_node::ServiceConfig>(value).is_err());
    }
    for invalid in [serde_json::Value::Null, serde_json::json!({"timeout": 600})] {
        let mut value = deployment_json();
        value["plugins"] = invalid;
        assert!(serde_json::from_value::<ora_node::ServiceConfig>(value).is_err());
    }
}

/// The default and configured policy reach a real installation, preserving retry/size constraints.
#[tokio::test]
async fn deployment_plugin_timing_reaches_installer_download_request() {
    let _logging = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::level_filters::LevelFilter::TRACE)
            .with_test_writer()
            .finish(),
    );
    for (plugins, attempt, total) in [
        (serde_json::json!({}), 60, 250),
        (serde_json::json!({"download_timeout_seconds": 10}), 10, 100),
        (
            serde_json::json!({"download_timeout_seconds": 600}),
            600,
            1870,
        ),
        (
            serde_json::json!({"download_timeout_seconds": 1200}),
            1200,
            3670,
        ),
    ] {
        let mut value = deployment_json();
        value["plugins"] = plugins;
        let config: ora_node::ServiceConfig = serde_json::from_value(value).unwrap();
        let root = tempfile::tempdir().unwrap();
        let source = archive(root.path(), "1.0.0");
        let observed = Arc::new(Mutex::new(Vec::new()));
        let installer = PluginInstaller::with_config(
            root.path().join("home"),
            PolicyDownload {
                archive: source.clone(),
                observed: observed.clone(),
            },
            /*host_target*/ None,
            config.plugins,
        );
        assert_eq!(
            installer.execute(&command(&source, "1.0.0"), &node()).await,
            result(PluginItemOutcome::Installed {
                version: PluginVersion::new("1.0.0")
            })
        );
        let mut expected = DownloadOptions::default();
        expected.connect_timeout = Some(std::time::Duration::from_secs(10));
        expected.per_attempt_timeout = Some(std::time::Duration::from_secs(attempt));
        expected.total_timeout = Some(std::time::Duration::from_secs(total));
        expected.max_retries = 2;
        expected.max_bytes = Some(512 * 1024 * 1024);
        assert_eq!(*observed.lock().unwrap(), vec![expected]);
    }
}

/// Supplies a real archive through the downloader boundary while counting duplicate transfers.
struct ArchiveDownload {
    archive: PathBuf,
    count: Arc<AtomicUsize>,
}
impl HttpDownload for ArchiveDownload {
    /// Retains production checksum verification and atomic download behavior.
    async fn download(
        &self,
        mut request: DownloadRequest,
    ) -> Result<DownloadOutcome, DownloadError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        request.source = DownloadSource::Local(self.archive.clone());
        LocalFileDownloader.download(request).await
    }
}

/// The archive has installed-form metadata and an entrypoint that must never execute on install.
fn archive(root: &std::path::Path, version: &str) -> PathBuf {
    let path = root.join(format!("{version}.orax"));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for (name, contents) in [
        (
            "orax.toml",
            format!(
                "resolver = 1\nidentifier = \"ora-space.echo\"\nkind = \"agent\"\nversion = \"{version}\"\ndescription = \"test\"\n"
            ),
        ),
        (
            "main.js",
            "throw new Error('installation must not execute plugins');".into(),
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    path
}

/// A complete universal input preserves Cloud's exact version and digest.
fn command(archive: &std::path::Path, version: &str) -> PluginCommand {
    PluginCommand::Install(InstallPluginsMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new("op"),
        execution_id: ExecutionId::new("execution"),
        payload: InstallPlugins {
            spec: InstallPluginsSpec {
                node_id: NodeId::new("node"),
                plugins: vec![PluginInstall {
                    plugin_id: PluginId::new("official/ora-space.echo"),
                    version: PluginVersion::new(version),
                    release: PluginRelease::Universal {
                        download: PluginDownload {
                            url: "https://example.com/package.orax".into(),
                            sha256: Sha256Digest::new(
                                ora_utils::hash::sha256_file(archive).unwrap(),
                            ),
                        },
                    },
                }],
            },
        },
    })
}

/// Results retain the originating Node process identity and all item outcomes.
fn result(outcome: PluginItemOutcome) -> PluginExecutionResult {
    PluginExecutionResult::PluginsCompleted(PluginsCompleted {
        node: node(),
        items: vec![PluginItemResult {
            plugin_id: PluginId::new("official/ora-space.echo"),
            outcome,
        }],
    })
}

/// Supplies the process identity independent of storage and downloads.
fn node() -> NodeRuntimeIdentity {
    NodeRuntimeIdentity {
        node_id: NodeId::new("node"),
        incarnation_id: NodeIncarnationId::new("incarnation"),
    }
}

/// Exact versions are idempotent; active use prevents replacement/removal and data survives.
#[tokio::test]
async fn install_retry_upgrade_and_remove_respect_use_leases() {
    let _logging = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::level_filters::LevelFilter::TRACE)
            .with_test_writer()
            .finish(),
    );
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let source = archive(root.path(), "1.0.0");
    let count = Arc::new(AtomicUsize::new(0));
    let installer = PluginInstaller::new(
        home.clone(),
        ArchiveDownload {
            archive: source.clone(),
            count: count.clone(),
        },
        /*host_target*/ None,
    );
    installer.recover().unwrap();
    let input = command(&source, "1.0.0");
    let installed = result(PluginItemOutcome::Installed {
        version: PluginVersion::new("1.0.0"),
    });
    assert_eq!(installer.execute(&input, &node()).await, installed);
    assert_eq!(installer.execute(&input, &node()).await, installed);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let catalog = installer.catalog();
    let id = PluginId::new("official/ora-space.echo");
    let old = catalog
        .installed(&id, &PluginVersion::new("1.0.0"))
        .unwrap();
    let lease = catalog.lease(&id);
    assert_eq!(
        installer.execute(&command(&source, "2.0.0"), &node()).await,
        result(PluginItemOutcome::Failed {
            failure: PluginFailureCode::PluginInUse
        })
    );
    let remove = PluginCommand::Remove(RemovePluginsMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new("remove-op"),
        execution_id: ExecutionId::new("remove-execution"),
        payload: RemovePlugins {
            spec: RemovePluginsSpec {
                node_id: NodeId::new("node"),
                plugins: vec![PluginRemoval {
                    plugin_id: id,
                    version: PluginVersion::new("1.0.0"),
                }],
            },
        },
    });
    assert_eq!(
        installer.execute(&remove, &node()).await,
        result(PluginItemOutcome::Failed {
            failure: PluginFailureCode::PluginInUse
        })
    );
    drop(lease);
    let next = archive(root.path(), "2.0.0");
    std::fs::copy(&next, &source).unwrap();
    assert_eq!(
        installer.execute(&command(&source, "2.0.0"), &node()).await,
        result(PluginItemOutcome::Installed {
            version: PluginVersion::new("2.0.0")
        })
    );
    assert!(!old.exists());
    assert!(
        catalog
            .installed(
                &PluginId::new("official/ora-space.echo"),
                &PluginVersion::new("2.0.0")
            )
            .is_some()
    );
    let data = home
        .join("plugins")
        .join("data")
        .join("official")
        .join("ora-space.echo");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("user-data"), "keep").unwrap();
    assert_eq!(
        installer.execute(&remove, &node()).await,
        result(PluginItemOutcome::Removed {})
    );
    assert_eq!(
        installer.execute(&remove, &node()).await,
        result(PluginItemOutcome::Removed {})
    );
    assert!(!old.exists());
    assert_eq!(
        std::fs::read_to_string(data.join("user-data")).unwrap(),
        "keep"
    );
}

/// Digest and identity errors cannot publish packages or leave temporary archives behind.
#[tokio::test]
async fn invalid_packages_leave_no_discoverable_installation() {
    let _logging = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::level_filters::LevelFilter::TRACE)
            .with_test_writer()
            .finish(),
    );
    let root = tempfile::tempdir().unwrap();
    let source = archive(root.path(), "1.0.0");
    let home = root.path().join("home");
    let installer = PluginInstaller::new(
        home.clone(),
        ArchiveDownload {
            archive: source.clone(),
            count: Arc::default(),
        },
        /*host_target*/ None,
    );
    assert_eq!(
        installer.execute(&command(&source, "2.0.0"), &node()).await,
        result(PluginItemOutcome::Failed {
            failure: PluginFailureCode::InvalidPackage
        })
    );
    let PluginCommand::Install(mut input) = command(&source, "1.0.0") else {
        unreachable!()
    };
    let PluginRelease::Universal { download } = &mut input.payload.spec.plugins[0].release else {
        unreachable!()
    };
    download.sha256 = Sha256Digest::new("0".repeat(64));
    assert_eq!(
        installer
            .execute(&PluginCommand::Install(input), &node())
            .await,
        result(PluginItemOutcome::Failed {
            failure: PluginFailureCode::ChecksumMismatch
        })
    );
    assert_eq!(
        std::fs::read_dir(home.join("plugins").join(".node-installs"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        std::fs::read_dir(ora_plugin_manager::installed_root(&home))
            .unwrap()
            .count(),
        0
    );
}

/// A linked root is rejected before writing outside the Workspace home.
#[tokio::test]
async fn linked_plugin_root_fails_the_execution_without_following_it() {
    let root = tempfile::tempdir().unwrap();
    let source = archive(root.path(), "1.0.0");
    let home = root.path().join("home");
    let outside = root.path().join("outside");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, home.join("plugins")).unwrap();
    let installer = PluginInstaller::new(
        home,
        ArchiveDownload {
            archive: source.clone(),
            count: Arc::default(),
        },
        /*host_target*/ None,
    );
    assert_eq!(
        installer.execute(&command(&source, "1.0.0"), &node()).await,
        PluginExecutionResult::PluginsFailed(PluginsFailed {
            node: node(),
            failure: PluginsFailureCode::PluginRootUnavailable
        })
    );
    assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
}

/// The production HTTP backend succeeds with verified bytes and bounds transient failure retries.
#[tokio::test]
async fn http_installation_and_three_failed_attempts_have_bounded_results() {
    let _logging = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::level_filters::LevelFilter::TRACE)
            .with_test_writer()
            .finish(),
    );
    for succeed in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let source = archive(root.path(), "1.0.0");
        let bytes = std::fs::read(&source).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/package.orax", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let requests = count.clone();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = [0u8; 4096];
                stream.read(&mut buffer).await.unwrap();
                requests.fetch_add(1, Ordering::SeqCst);
                let (status, body) = if succeed {
                    ("200 OK", bytes.as_slice())
                } else {
                    ("503 Service Unavailable", &[][..])
                };
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                stream.write_all(body).await.unwrap();
            }
        });
        let PluginCommand::Install(mut input) = command(&source, "1.0.0") else {
            unreachable!()
        };
        let PluginRelease::Universal { download } = &mut input.payload.spec.plugins[0].release
        else {
            unreachable!()
        };
        download.url = url;
        let installer = PluginInstaller::new(
            root.path().join("home"),
            ora_utils::http::ReqwestDownloader::new(ora_utils::http::ProxyConfig::default()),
            /*host_target*/ None,
        );
        let actual = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            installer.execute(&PluginCommand::Install(input), &node()),
        )
        .await
        .unwrap();
        assert_eq!(
            actual,
            result(if succeed {
                PluginItemOutcome::Installed {
                    version: PluginVersion::new("1.0.0"),
                }
            } else {
                PluginItemOutcome::Failed {
                    failure: PluginFailureCode::DownloadFailed,
                }
            })
        );
        assert_eq!(count.load(Ordering::SeqCst), if succeed { 1 } else { 3 });
        server.abort();
    }
}

/// Target selection is exact and never falls back to a different platform's package.
#[tokio::test]
async fn unmatched_target_never_downloads() {
    let root = tempfile::tempdir().unwrap();
    let source = archive(root.path(), "1.0.0");
    let PluginCommand::Install(mut input) = command(&source, "1.0.0") else {
        unreachable!()
    };
    let PluginRelease::Universal { download } = input.payload.spec.plugins[0].release.clone()
    else {
        unreachable!()
    };
    input.payload.spec.plugins[0].release = PluginRelease::Targets {
        targets: vec![PluginTargetDownload {
            target: "aarch64-apple-darwin".into(),
            download,
        }],
    };
    let count = Arc::new(AtomicUsize::new(0));
    let installer = PluginInstaller::new(
        root.path().join("home"),
        ArchiveDownload {
            archive: source,
            count: count.clone(),
        },
        Some(ora_plugin_manager::HookTarget::parse("x86_64-unknown-linux-gnu").unwrap()),
    );
    assert_eq!(
        installer
            .execute(&PluginCommand::Install(input), &node())
            .await,
        result(PluginItemOutcome::Failed {
            failure: PluginFailureCode::NoMatchingTarget
        })
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
}
