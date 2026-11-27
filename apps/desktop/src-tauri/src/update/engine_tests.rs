//! Tests for how the update engine wires one check's manifest, the running version, retained
//! packages, and confirmed installations together.

use crate::update::engine::{OfferedPackage, UpdateEngine, UpdateHost};
use crate::update::platform::InstallSupport;
use crate::update::release_notes::ReleaseNote;
use crate::update::tests::{
    TestVerifier, digest, manifest, only_entry, open_store, release_note, store_root,
};
use crate::update::{DesktopUpdateStatus, FailedOperation, UpdateError};
use pretty_assertions::assert_eq;
use semver::Version;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use url::Url;

/// A release offered by a scripted check, recording what was handed to its installer.
#[derive(Clone)]
struct FakeRelease {
    version: String,
    raw_json: Value,
    bytes: Vec<u8>,
    download_url: Url,
    signature: String,
    downloads: Arc<Mutex<usize>>,
    installed: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl FakeRelease {
    /// Offers `version` with the given manifest and a package signed by `TestVerifier`.
    fn new(version: &str, raw_json: Value) -> Self {
        let bytes = format!("package {version}").into_bytes();
        Self {
            version: version.to_owned(),
            raw_json,
            signature: digest(&bytes),
            bytes,
            download_url: Url::parse("https://updates.example/ora.AppImage").expect("source URL"),
            downloads: Arc::default(),
            installed: Arc::default(),
        }
    }
}

impl OfferedPackage for FakeRelease {
    fn version(&self) -> &str {
        &self.version
    }

    fn target(&self) -> &str {
        "test-target"
    }

    fn download_url(&self) -> &Url {
        &self.download_url
    }

    fn signature(&self) -> &str {
        &self.signature
    }

    fn raw_json(&self) -> &Value {
        &self.raw_json
    }

    fn download(
        &self,
        mut on_chunk: impl FnMut(usize, Option<u64>) + Send,
    ) -> impl Future<Output = Result<Vec<u8>, UpdateError>> + Send {
        *self.downloads.lock().expect("download count") += 1;
        on_chunk(self.bytes.len(), Some(self.bytes.len() as u64));
        let bytes = self.bytes.clone();
        async move { Ok(bytes) }
    }

    fn install(&self, bytes: Vec<u8>) -> Result<(), UpdateError> {
        self.installed
            .lock()
            .expect("installed packages")
            .push(bytes);
        Ok(())
    }
}

/// A host whose checks return scripted results in order.
struct FakeHost {
    running: Version,
    checks: Mutex<VecDeque<Result<Option<FakeRelease>, UpdateError>>>,
}

impl UpdateHost for FakeHost {
    type Release = FakeRelease;

    fn running_version(&self) -> Version {
        self.running.clone()
    }

    fn install_support(&self) -> InstallSupport {
        InstallSupport::SelfUpdating
    }

    fn check(&self) -> impl Future<Output = Result<Option<FakeRelease>, UpdateError>> + Send {
        let next = self
            .checks
            .lock()
            .expect("scripted checks")
            .pop_front()
            .expect("a check was scripted");
        async move { next }
    }

    fn publish(&self, _status: DesktopUpdateStatus) {}
}

/// Builds an engine over a fresh store in `home` that answers checks in order.
fn engine(
    home: &Path,
    running: &str,
    checks: Vec<Result<Option<FakeRelease>, UpdateError>>,
) -> UpdateEngine<FakeHost, TestVerifier> {
    UpdateEngine::new(
        FakeHost {
            running: Version::parse(running).expect("running version"),
            checks: Mutex::new(checks.into()),
        },
        open_store(home),
        TestVerifier,
    )
}

/// Returns the shared test note relabelled as `version`'s.
fn note(version: &str) -> ReleaseNote {
    ReleaseNote {
        version: version.to_owned(),
        ..release_note()
    }
}

/// Returns a manifest carrying exactly `notes`.
fn manifest_with(notes: &[ReleaseNote]) -> Value {
    json!({ "ora_release_notes": { "schema": 1, "versions": notes } })
}

/// Stands in for a failed manifest request.
fn unreachable() -> UpdateError {
    UpdateError::TrustConfiguration("endpoint unreachable".to_owned())
}

/// Overwrites the committed package so its digest no longer matches its record.
fn tamper_with_package(home: &Path) {
    let entry = only_entry(home);
    let payload = std::fs::read_dir(&entry)
        .expect("entry is readable")
        .map(|file| file.expect("entry file").path())
        .find(|path| path.file_name().is_some_and(|name| name != "record.json"))
        .expect("entry holds a payload");
    std::fs::write(payload, b"tampered").expect("payload is writable");
}

/// The running package's version, not the build's crate version, bounds the offered notes.
#[tokio::test]
async fn offered_notes_start_after_the_running_package_version() {
    let home = TempDir::new().expect("temp home");
    let engine = engine(
        home.path(),
        "0.3.0",
        vec![Ok(Some(FakeRelease::new("0.4.0", manifest())))],
    );

    engine.check_and_download().await;

    assert_eq!(
        engine.status(),
        DesktopUpdateStatus::Ready {
            version: "0.4.0".to_owned(),
            notes: vec![note("0.4.0")],
        }
    );
}

/// Every check offers the notes of its own manifest, even when an earlier one had others.
#[tokio::test]
async fn each_offer_carries_the_notes_of_its_own_check() {
    let home = TempDir::new().expect("temp home");
    let engine = engine(
        home.path(),
        "0.2.0",
        vec![
            Ok(Some(FakeRelease::new(
                "0.3.0",
                manifest_with(&[note("0.3.0")]),
            ))),
            Ok(Some(FakeRelease::new(
                "0.4.0",
                manifest_with(&[note("0.4.0"), note("0.3.0")]),
            ))),
        ],
    );

    engine.check_and_download().await;
    let first = engine.status();
    engine.check_and_download().await;

    assert_eq!(
        (first, engine.status()),
        (
            DesktopUpdateStatus::Ready {
                version: "0.3.0".to_owned(),
                notes: vec![note("0.3.0")],
            },
            DesktopUpdateStatus::Ready {
                version: "0.4.0".to_owned(),
                notes: vec![note("0.4.0"), note("0.3.0")],
            },
        )
    );
}

/// A failed check keeps the ready release only while its package still verifies on disk.
#[tokio::test]
async fn a_failed_check_keeps_a_ready_release_only_while_its_package_verifies() {
    let intact = TempDir::new().expect("temp home");
    let tampered = TempDir::new().expect("temp home");
    let offer = || {
        vec![
            Ok(Some(FakeRelease::new("0.3.0", manifest()))),
            Err(unreachable()),
        ]
    };
    let kept = engine(intact.path(), "0.2.0", offer());
    let discarded = engine(tampered.path(), "0.2.0", offer());

    kept.check_and_download().await;
    discarded.check_and_download().await;
    tamper_with_package(tampered.path());
    kept.check_and_download().await;
    discarded.check_and_download().await;

    assert_eq!(
        (kept.status(), discarded.status()),
        (
            DesktopUpdateStatus::Ready {
                version: "0.3.0".to_owned(),
                notes: vec![note("0.3.0")],
            },
            DesktopUpdateStatus::Failed {
                operation: FailedOperation::Check,
                message: "the updater trust configuration is invalid: endpoint unreachable"
                    .to_owned(),
            },
        )
    );
}

/// Installation reaches the state only for the version the user confirmed.
#[tokio::test]
async fn installation_is_for_the_confirmed_version_only() {
    let home = TempDir::new().expect("temp home");
    let release = FakeRelease::new("0.3.0", manifest());
    let installed = release.installed.clone();
    let engine = engine(home.path(), "0.2.0", vec![Ok(Some(release))]);
    engine.check_and_download().await;

    let replaced = engine.install("0.2.9").await;
    let after_replaced = (engine.status(), installed.lock().expect("installed").len());
    engine.install("0.3.0").await.expect("installation starts");

    assert_eq!(
        (
            matches!(replaced, Err(UpdateError::ReleaseReplaced)),
            after_replaced,
            engine.status(),
            installed.lock().expect("installed").clone(),
        ),
        (
            true,
            (
                DesktopUpdateStatus::Ready {
                    version: "0.3.0".to_owned(),
                    notes: vec![note("0.3.0")],
                },
                0,
            ),
            DesktopUpdateStatus::Current,
            vec![b"package 0.3.0".to_vec()],
        )
    );
}

/// A confirmed package that no longer verifies is reported as an installation failure for it.
#[tokio::test]
async fn a_package_that_fails_its_last_verification_is_an_install_failure() {
    let home = TempDir::new().expect("temp home");
    let release = FakeRelease::new("0.3.0", manifest());
    let installed = release.installed.clone();
    let engine = engine(home.path(), "0.2.0", vec![Ok(Some(release))]);
    engine.check_and_download().await;
    tamper_with_package(home.path());

    let result = engine.install("0.3.0").await;

    assert_eq!(
        (
            matches!(result, Err(UpdateError::CachedArtifactChanged)),
            engine.status(),
            installed.lock().expect("installed").len(),
        ),
        (
            true,
            DesktopUpdateStatus::Failed {
                operation: FailedOperation::Install {
                    version: "0.3.0".to_owned(),
                },
                message: "cached update digest changed".to_owned(),
            },
            0,
        )
    );
}

/// Notes live only in memory: the store keeps none, and a later process shows the new manifest's.
#[tokio::test]
async fn manifest_notes_are_never_persisted() {
    let home = TempDir::new().expect("temp home");
    let first = engine(
        home.path(),
        "0.2.0",
        vec![Ok(Some(FakeRelease::new("0.3.0", manifest())))],
    );
    first.check_and_download().await;
    let stored = stored_text(&store_root(home.path()));
    drop(first);

    let recovered = FakeRelease::new("0.3.0", manifest_with(&[]));
    let downloads = recovered.downloads.clone();
    let second = engine(home.path(), "0.2.0", vec![Ok(Some(recovered))]);
    second.check_and_download().await;

    assert_eq!(
        (
            stored.contains("修复") || stored.contains("Fixed"),
            second.status(),
            *downloads.lock().expect("download count"),
        ),
        (
            false,
            DesktopUpdateStatus::Ready {
                version: "0.3.0".to_owned(),
                notes: Vec::new(),
            },
            0,
        )
    );
}

/// Concatenates every file below `root`, lossily decoded, so a test can search what was written.
fn stored_text(root: &Path) -> String {
    let mut text = String::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).expect("store is readable") {
            let path = entry.expect("store entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                text.push_str(&String::from_utf8_lossy(
                    &std::fs::read(path).expect("store file is readable"),
                ));
            }
        }
    }
    text
}
