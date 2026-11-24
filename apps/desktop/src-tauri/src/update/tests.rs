//! Tests for artifact recovery, runtime state invariants, proxy URLs, and the webview contract.

use crate::update::artifact_store::{
    ArtifactDescriptor, ArtifactVerifier, StoredArtifact, UpdateArtifactStore,
};
use crate::update::release_notes::{ReleaseNote, ReleaseNoteCategory, ReleaseNoteEntry};
use crate::update::service::proxy_url;
use crate::update::state::{ReadyUpdate, RuntimeUpdateState};
use crate::update::{DesktopUpdateStatus, ManualUpdateReason};
use ora_application::NetworkProxySettings;
use ora_utils::hash::sha256_reader;
use pretty_assertions::assert_eq;
use semver::Version;
use serde_json::{Value, json};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use url::Url;

struct TestVerifier;

impl ArtifactVerifier for TestVerifier {
    /// Returns the stable fake trust root shared by every test descriptor.
    fn trust_root_fingerprint(&self) -> &str {
        "test-trust-root"
    }

    /// Accepts the SHA-256 spelling as deterministic local signature evidence.
    fn verify(&self, bytes: &[u8], encoded_signature: &str) -> bool {
        digest(bytes) == encoded_signature
    }
}

/// Computes the deterministic test signature accepted by `TestVerifier`.
fn digest(bytes: &[u8]) -> String {
    sha256_reader(Cursor::new(bytes)).expect("in-memory hashing succeeds")
}

/// Returns the root of the versioned Desktop update store below the Ora home.
fn store_root(home_directory: &Path) -> PathBuf {
    home_directory
        .join("cache")
        .join("desktop-updates")
        .join("v2")
}

/// Opens a store as a build older than every release used by these tests.
fn open_store(home: &Path) -> UpdateArtifactStore {
    UpdateArtifactStore::open(home, &Version::parse("0.1.0").expect("current version"))
        .expect("store opens")
}

/// Builds fresh manifest evidence whose signature is bound to `bytes`.
fn descriptor(version: &str, bytes: &[u8]) -> ArtifactDescriptor {
    ArtifactDescriptor::new(
        version.to_owned(),
        "test-target".to_owned(),
        Url::parse("https://updates.example/ora.AppImage").expect("source URL"),
        digest(bytes),
        &TestVerifier,
    )
    .expect("descriptor builds")
}

/// Returns the sole committed entry path after a test stores one artifact.
fn only_entry(home: &Path) -> PathBuf {
    let entries = store_root(home).join("entries");
    let paths = std::fs::read_dir(entries)
        .expect("entries are readable")
        .map(|entry| entry.expect("entry").path())
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 1);
    paths.into_iter().next().expect("one entry")
}

/// Commits an artifact into the store and returns its verified reference.
async fn commit_release(
    store: &UpdateArtifactStore,
    version: &str,
    bytes: &[u8],
) -> (ArtifactDescriptor, StoredArtifact) {
    let descriptor = descriptor(version, bytes);
    let artifact = store
        .commit(&descriptor, bytes, &TestVerifier)
        .await
        .expect("commit succeeds");
    (descriptor, artifact)
}

/// Builds notes as the update service attaches them to an offered release.
fn release_note() -> ReleaseNote {
    ReleaseNote {
        version: "0.3.0".to_owned(),
        date: "2026-12-10".to_owned(),
        entries: vec![ReleaseNoteEntry {
            category: ReleaseNoteCategory::Fix,
            zh_cn: "修复".to_owned(),
            en_us: "Fixed".to_owned(),
        }],
    }
}

/// Verifies a commit records full identity and leaves no interrupted staging directory.
#[tokio::test]
async fn commit_publishes_an_identity_addressed_entry() {
    let home = TempDir::new().expect("temp home");
    let store = open_store(home.path());
    let bytes = b"signed-package".as_slice();
    let (descriptor, _) = commit_release(&store, "0.3.0", bytes).await;
    let entry = only_entry(home.path());
    let record: Value = serde_json::from_slice(
        &std::fs::read(entry.join("record.json")).expect("record is readable"),
    )
    .expect("record parses");
    let payload_file_name = record["payload"]["fileName"]
        .as_str()
        .expect("payload file name")
        .to_owned();
    assert_eq!(
        (
            record,
            std::fs::read(entry.join(&payload_file_name)).expect("payload is readable"),
            std::fs::read_dir(store_root(home.path()).join("staging"))
                .expect("staging is readable")
                .count(),
        ),
        (
            json!({
                "schemaVersion": 2,
                "identity": serde_json::to_value(&descriptor.identity).expect("identity serializes"),
                "sourceUrl": "https://updates.example/ora.AppImage",
                "originalFileName": "ora.AppImage",
                "payload": {
                    "fileName": payload_file_name,
                    "byteLength": bytes.len(),
                    "sha256": digest(bytes),
                }
            }),
            bytes.to_vec(),
            0,
        )
    );
}

/// Simulates a second process and confirms it can recover bytes without committing again.
#[tokio::test]
async fn matching_artifact_is_recovered_after_reopening_the_store() {
    let home = TempDir::new().expect("temp home");
    let bytes = b"signed-package".as_slice();
    let store = open_store(home.path());
    let (descriptor, committed) = commit_release(&store, "0.3.0", bytes).await;
    drop(store);

    let reopened = open_store(home.path());
    let recovered = reopened
        .find_verified(&descriptor, &TestVerifier)
        .await
        .expect("recovery succeeds")
        .expect("artifact is recovered");

    assert_eq!(
        (
            recovered.clone(),
            reopened
                .read_verified(&recovered, &descriptor, &TestVerifier)
                .await
                .expect("artifact remains verified"),
        ),
        (committed, bytes.to_vec())
    );
}

/// Ensures local package tampering removes the entry instead of advertising it as ready.
#[tokio::test]
async fn changed_package_is_rejected_and_removed() {
    let home = TempDir::new().expect("temp home");
    let store = open_store(home.path());
    let bytes = b"signed-package".as_slice();
    let (descriptor, artifact) = commit_release(&store, "0.3.0", bytes).await;
    std::fs::write(&artifact.payload_path, b"changed-package").expect("payload is writable");

    assert_eq!(
        store
            .find_verified(&descriptor, &TestVerifier)
            .await
            .expect("invalid cache is recoverable"),
        None
    );
    assert!(!artifact.entry_path.exists());
}

/// Ensures a failed replacement commit leaves the previously verified artifact untouched.
#[tokio::test]
async fn failed_replacement_preserves_the_existing_entry() {
    let home = TempDir::new().expect("temp home");
    let store = open_store(home.path());
    let old_bytes = b"old-signed-package".as_slice();
    let (old_descriptor, old_artifact) = commit_release(&store, "0.3.0", old_bytes).await;
    let mut invalid_descriptor = descriptor("0.4.0", b"different-package");
    invalid_descriptor.encoded_signature = "not-a-valid-signature".to_owned();

    assert!(
        store
            .commit(&invalid_descriptor, b"new-package", &TestVerifier)
            .await
            .is_err()
    );
    assert_eq!(
        (
            only_entry(home.path()),
            store
                .find_verified(&old_descriptor, &TestVerifier)
                .await
                .expect("old lookup succeeds"),
        ),
        (old_artifact.entry_path.clone(), Some(old_artifact))
    );
}

/// Confirms a replacement removes the old entry only after the new artifact is committed.
#[tokio::test]
async fn successful_replacement_prunes_the_previous_entry() {
    let home = TempDir::new().expect("temp home");
    let store = open_store(home.path());
    let (_, old_artifact) = commit_release(&store, "0.3.0", b"old-signed-package").await;

    let (new_descriptor, new_artifact) =
        commit_release(&store, "0.4.0", b"new-signed-package").await;

    assert_eq!(
        (
            old_artifact.entry_path.exists(),
            only_entry(home.path()),
            store
                .find_verified(&new_descriptor, &TestVerifier)
                .await
                .expect("new lookup succeeds"),
        ),
        (false, new_artifact.entry_path.clone(), Some(new_artifact),)
    );
}

/// Confirms startup clears interrupted writes from the current versioned store.
#[test]
fn open_cleans_interrupted_writes() {
    let home = TempDir::new().expect("temp home");
    let interrupted = store_root(home.path()).join("staging").join("download-old");
    std::fs::create_dir_all(&interrupted).expect("staging fixture");
    std::fs::write(interrupted.join("payload.AppImage"), b"partial").expect("partial payload");

    let _store = open_store(home.path());

    assert_eq!(
        std::fs::read_dir(store_root(home.path()).join("staging"))
            .expect("staging is readable")
            .count(),
        0
    );
}

/// Verifies a running build discards a committed release it already includes.
#[tokio::test]
async fn open_discards_a_superseded_release() {
    let home = TempDir::new().expect("temp home");
    let store = open_store(home.path());
    commit_release(&store, "0.3.0", b"signed-package").await;
    drop(store);

    let _reopened = UpdateArtifactStore::open(
        home.path(),
        &Version::parse("0.3.0").expect("current version"),
    )
    .expect("store reopens");

    assert_eq!(
        std::fs::read_dir(store_root(home.path()).join("entries"))
            .expect("entries are readable")
            .count(),
        0
    );
}

/// Verifies runtime `Ready` owns all data needed to enter and recover from installation.
#[tokio::test]
async fn runtime_state_keeps_status_and_installable_data_together() {
    let home = TempDir::new().expect("temp home");
    let store = open_store(home.path());
    let (descriptor, artifact) = commit_release(&store, "0.3.0", b"signed-package").await;
    let ready = ReadyUpdate {
        installer: "test-installer".to_owned(),
        descriptor,
        artifact,
        notes: vec![release_note()],
    };
    let mut state = RuntimeUpdateState::Ready(ready);

    assert_eq!(
        state.status(),
        DesktopUpdateStatus::Ready {
            version: "0.3.0".to_owned(),
            notes: vec![release_note()],
        }
    );
    let installing = state.begin_install().expect("ready state installs");
    assert_eq!(
        (state.status(), installing.installer),
        (
            DesktopUpdateStatus::Installing {
                version: "0.3.0".to_owned()
            },
            "test-installer".to_owned(),
        )
    );
}

/// Verifies proxy credentials are encoded into the updater URL.
#[test]
fn proxy_url_carries_the_host_port_and_credentials() {
    let url = proxy_url(&NetworkProxySettings {
        host: "proxy.internal".to_owned(),
        port: 8080,
        username: Some("agent".to_owned()),
        password: Some("s3cr3t".to_owned()),
    })
    .expect("proxy URL builds");

    assert_eq!(url.as_str(), "http://agent:s3cr3t@proxy.internal:8080/");
}

/// Verifies absent proxy credentials remain absent instead of becoming empty URL fields.
#[test]
fn proxy_url_omits_credentials_that_are_not_configured() {
    let url = proxy_url(&NetworkProxySettings {
        host: "proxy.internal".to_owned(),
        port: 3128,
        username: None,
        password: None,
    })
    .expect("proxy URL builds");

    assert_eq!(url.as_str(), "http://proxy.internal:3128/");
}

/// The webview switches on `kind` and reads payload fields directly, making this serialized shape
/// part of the platform contract in `packages/app-shell/src/platform/types.ts`.
#[test]
fn status_serializes_to_the_shape_the_webview_consumes() {
    let statuses = vec![
        DesktopUpdateStatus::Current,
        DesktopUpdateStatus::Checking,
        DesktopUpdateStatus::Downloading {
            version: "0.3.0".to_owned(),
            downloaded: 1024,
            total: Some(4096),
        },
        DesktopUpdateStatus::Ready {
            version: "0.3.0".to_owned(),
            notes: vec![release_note()],
        },
        DesktopUpdateStatus::ManualUpdate {
            version: "0.3.0".to_owned(),
            reason: ManualUpdateReason::SystemPackage,
            notes: Vec::new(),
        },
        DesktopUpdateStatus::Installing {
            version: "0.3.0".to_owned(),
        },
        DesktopUpdateStatus::Failed {
            message: "endpoint unreachable".to_owned(),
        },
    ];

    assert_eq!(
        serde_json::to_value(&statuses).expect("statuses serialize"),
        json!([
            { "kind": "current" },
            { "kind": "checking" },
            { "kind": "downloading", "version": "0.3.0", "downloaded": 1024, "total": 4096 },
            {
                "kind": "ready",
                "version": "0.3.0",
                "notes": [{
                    "version": "0.3.0",
                    "date": "2026-12-10",
                    "entries": [{ "category": "fix", "zh-CN": "修复", "en-US": "Fixed" }],
                }],
            },
            {
                "kind": "manual_update",
                "version": "0.3.0",
                "reason": "system_package",
                "notes": [],
            },
            { "kind": "installing", "version": "0.3.0" },
            { "kind": "failed", "message": "endpoint unreachable" },
        ]) as Value
    );
}

/// Verifies Rust manual-update reasons keep the discriminants consumed by TypeScript.
#[test]
fn manual_update_reasons_serialize_as_the_webview_discriminants() {
    assert_eq!(
        serde_json::to_value([
            ManualUpdateReason::SystemPackage,
            ManualUpdateReason::UnpackagedBinary,
        ])
        .expect("reasons serialize"),
        json!(["system_package", "unpackaged_binary"]) as Value
    );
}
