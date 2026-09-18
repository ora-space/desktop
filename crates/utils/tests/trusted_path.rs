#![cfg(unix)]

use ora_utils::path::{TrustedPathKind, open_trusted_path};
use pretty_assertions::assert_eq;
use std::fs;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

/// Reading a trusted file pins the verified inode and refuses symbolic-link replacements.
#[test]
fn trusted_file_is_opened_without_following_links() {
    // The checkout and system temp directory may be group-writable; the owner-controlled home
    // provides trusted ancestors without relaxing the production check for test fixtures.
    let fixture_parent = std::path::PathBuf::from(
        std::env::var_os("HOME").unwrap_or_else(|| panic!("HOME is required")),
    )
    .canonicalize()
    .unwrap_or_else(|e| panic!("fixture parent: {e}"));
    let root = tempfile::tempdir_in(fixture_parent).unwrap_or_else(|e| panic!("tempdir: {e}"));
    fs::set_permissions(root.path(), fs::Permissions::from_mode(/*mode*/ 0o700))
        .unwrap_or_else(|e| panic!("directory permissions: {e}"));
    let owner = fs::metadata(root.path())
        .unwrap_or_else(|e| panic!("metadata: {e}"))
        .uid();
    let path = root.path().join("config");
    fs::write(&path, "trusted").unwrap_or_else(|e| panic!("write: {e}"));
    fs::set_permissions(&path, fs::Permissions::from_mode(/*mode*/ 0o600))
        .unwrap_or_else(|e| panic!("permissions: {e}"));
    let mut file = open_trusted_path(&path, owner, TrustedPathKind::File)
        .unwrap_or_else(|e| panic!("open: {e}"));
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .unwrap_or_else(|e| panic!("read: {e}"));
    assert_eq!(contents, "trusted");
    // Opening a write-only control/configuration file must not truncate it.
    let writable = open_trusted_path(&path, owner, TrustedPathKind::WritableFile)
        .unwrap_or_else(|e| panic!("open writable: {e}"));
    drop(writable);
    assert_eq!(
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read: {e}")),
        "trusted"
    );
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap_or_else(|e| panic!("symlink: {e}"));
    assert!(open_trusted_path(&link, owner, TrustedPathKind::File).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(/*mode*/ 0o666))
        .unwrap_or_else(|e| panic!("permissions: {e}"));
    assert!(open_trusted_path(&path, owner, TrustedPathKind::File).is_err());
}
