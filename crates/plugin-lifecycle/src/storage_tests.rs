//! Tests for the `ora/storage/*` handler: containment, `web-profile` denial, round trips, and
//! the wire shapes a plugin observes.

use crate::storage::{
    PluginStorage, STORAGE_LIST_METHOD, STORAGE_READ_METHOD, STORAGE_REMOVE_METHOD,
    STORAGE_WRITE_METHOD,
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ora_plugin_runtime::{HostRequestError, HostRequestHandler};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

/// Builds a data directory with the host-created children and one stray file outside it.
fn fixture() -> (TempDir, PluginStorage) {
    let temp_dir = TempDir::new().expect("create data root");
    let data_dir = temp_dir.path().join("data");
    fs::create_dir_all(data_dir.join("downloads")).expect("create downloads");
    fs::create_dir_all(data_dir.join("web-profile").join("market")).expect("create web profile");
    fs::write(data_dir.join("downloads").join("skill.zip"), b"zip bytes").expect("write download");
    fs::write(
        data_dir.join("web-profile").join("market").join("cookies"),
        b"secret",
    )
    .expect("write cookies");
    fs::write(temp_dir.path().join("outside.txt"), b"outside").expect("write outside file");
    (temp_dir, PluginStorage::new(data_dir))
}

/// The `data.kind` classification of one failed call, as a plugin would branch on it.
fn kind_of(error: &HostRequestError) -> String {
    error.data()["kind"].as_str().unwrap_or_default().to_owned()
}

/// Every escape spelling, the host-owned profile directory, and symlinks are refused with
/// `invalid_path`, and none of them touches the file outside the data directory.
#[tokio::test]
async fn refuses_escapes_web_profile_and_symlinks() {
    let (temp_dir, storage) = fixture();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        temp_dir.path().join("outside.txt"),
        temp_dir.path().join("data").join("link.txt"),
    )
    .expect("create symlink");

    let mut kinds = Vec::new();
    for (method, path) in [
        (STORAGE_READ_METHOD, "../outside.txt"),
        (STORAGE_READ_METHOD, "/etc/passwd"),
        (STORAGE_LIST_METHOD, "web-profile"),
        (STORAGE_READ_METHOD, "web-profile/market/cookies"),
        (STORAGE_WRITE_METHOD, "web-profile/market/injected"),
        (STORAGE_REMOVE_METHOD, "web-profile"),
        (STORAGE_WRITE_METHOD, "../escaped.txt"),
        (STORAGE_REMOVE_METHOD, ""),
        (STORAGE_WRITE_METHOD, ""),
        #[cfg(unix)]
        (STORAGE_READ_METHOD, "link.txt"),
        #[cfg(unix)]
        (STORAGE_WRITE_METHOD, "link.txt"),
    ] {
        let error = storage
            .handle(method, json!({ "path": path, "bytes_base64": "" }))
            .await
            .expect_err(path);
        kinds.push((method, path, kind_of(&error)));
    }

    let expected = kinds
        .iter()
        .map(|(method, path, _)| (*method, *path, "invalid_path".to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(
        (
            kinds,
            fs::read(temp_dir.path().join("outside.txt")).expect("outside file intact"),
            temp_dir.path().join("escaped.txt").exists(),
            temp_dir
                .path()
                .join("data")
                .join("web-profile")
                .join("market")
                .join("injected")
                .exists(),
        ),
        (expected, b"outside".to_vec(), false, false),
    );
}

/// Write creates parents, read returns the same bytes, list shows the entry, remove deletes it.
#[tokio::test]
async fn round_trips_a_file_through_write_read_list_remove() {
    let (_temp_dir, storage) = fixture();
    let payload = BASE64.encode(b"{\"installed\":[\"abc\"]}");

    let written = storage
        .handle(
            STORAGE_WRITE_METHOD,
            json!({ "path": "state/index.json", "bytes_base64": payload }),
        )
        .await;
    let read = storage
        .handle(STORAGE_READ_METHOD, json!({ "path": "state/index.json" }))
        .await;
    let listed = storage
        .handle(STORAGE_LIST_METHOD, json!({ "path": "state" }))
        .await;
    let removed = storage
        .handle(STORAGE_REMOVE_METHOD, json!({ "path": "state" }))
        .await;
    let after_remove = storage
        .handle(STORAGE_READ_METHOD, json!({ "path": "state/index.json" }))
        .await
        .map_err(|error| kind_of(&error));

    assert_eq!(
        (written, read, listed, removed, after_remove),
        (
            Ok(json!({})),
            Ok(json!({ "bytes_base64": payload })),
            Ok(json!({
                "entries": [
                    { "name": "index.json", "kind": "file", "size_bytes": 21 },
                ],
            })),
            Ok(json!({})),
            Err("not_found".to_owned()),
        ),
    );
}

/// Listing the root shows `downloads/` but never `web-profile/`, and entries are sorted.
#[tokio::test]
async fn root_listing_hides_web_profile() {
    let (_temp_dir, storage) = fixture();
    storage
        .handle(
            STORAGE_WRITE_METHOD,
            json!({ "path": "a.txt", "bytes_base64": BASE64.encode(b"hi") }),
        )
        .await
        .expect("write a.txt");

    let listed = storage
        .handle(STORAGE_LIST_METHOD, json!({ "path": "" }))
        .await;
    let downloads = storage
        .handle(STORAGE_LIST_METHOD, json!({ "path": "downloads" }))
        .await;

    assert_eq!(
        (listed, downloads),
        (
            Ok(json!({
                "entries": [
                    { "name": "a.txt", "kind": "file", "size_bytes": 2 },
                    { "name": "downloads", "kind": "directory", "size_bytes": 0 },
                ],
            })),
            Ok(json!({
                "entries": [
                    { "name": "skill.zip", "kind": "file", "size_bytes": 9 },
                ],
            })),
        ),
    );
}

/// Unknown methods, malformed params, and oversized content are each reported distinctly.
#[tokio::test]
async fn reports_unknown_methods_bad_params_and_size_limits() {
    let (temp_dir, storage) = fixture();
    let big = temp_dir.path().join("data").join("big.bin");
    let file = fs::File::create(&big).expect("create big file");
    file.set_len(crate::storage::MAX_STORAGE_FILE_BYTES + 1)
        .expect("extend big file");

    let unknown = storage
        .handle("ora/storage/stat", json!({ "path": "a" }))
        .await;
    let missing_path = storage
        .handle(STORAGE_READ_METHOD, json!({}))
        .await
        .map_err(|error| kind_of(&error));
    let bad_base64 = storage
        .handle(
            STORAGE_WRITE_METHOD,
            json!({ "path": "x.bin", "bytes_base64": "%%%" }),
        )
        .await
        .map_err(|error| kind_of(&error));
    let too_large = storage
        .handle(STORAGE_READ_METHOD, json!({ "path": "big.bin" }))
        .await
        .map_err(|error| kind_of(&error));
    let missing_file = storage
        .handle(STORAGE_READ_METHOD, json!({ "path": "downloads/nope.zip" }))
        .await
        .map_err(|error| kind_of(&error));

    assert_eq!(
        (unknown, missing_path, bad_base64, too_large, missing_file),
        (
            Err(HostRequestError::method_not_found("ora/storage/stat")),
            Err("invalid_params".to_owned()),
            Err("invalid_params".to_owned()),
            Err("too_large".to_owned()),
            Err("not_found".to_owned()),
        ),
    );
}
