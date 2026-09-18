#![cfg(target_os = "linux")]

use std::fs::{self, File};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use ora_utils::{
    fs::LinuxFilesystem,
    path::{TrustedPathKind, open_private_path},
};
use pretty_assertions::assert_eq;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Uses the owner-controlled home only for test fixtures, as in the existing trusted-path tests.
fn directory() -> Result<tempfile::TempDir, std::io::Error> {
    let parent = std::env::var_os("HOME")
        .ok_or_else(|| std::io::Error::other("HOME required for test fixtures"))?;
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
        .tempdir_in(Path::new(&parent).canonicalize()?)
}

/// Owner-private targets are readable but wrong owners and permissive modes are never repaired.
#[test]
fn private_path_requires_exact_owner_and_private_permissions() -> TestResult {
    let directory = directory()?;
    // SAFETY: geteuid only queries the effective process identity.
    let owner = unsafe { libc::geteuid() };
    open_private_path(directory.path(), owner, TrustedPathKind::Directory)?;
    let path = directory.path().join("value");
    fs::write(&path, b"private")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(/*mode*/ 0o600))?;
    open_private_path(&path, owner, TrustedPathKind::File)?;
    assert!(open_private_path(&path, owner.wrapping_add(1), TrustedPathKind::File).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(/*mode*/ 0o640))?;
    assert!(open_private_path(&path, owner, TrustedPathKind::File).is_err());
    assert_eq!(fs::read(&path)?, b"private");
    Ok(())
}

/// Alternate links and symlinked ancestors cannot alias accepted private state.
#[test]
fn private_path_rejects_symlinks_and_multiple_hard_links() -> TestResult {
    let directory = directory()?;
    // SAFETY: geteuid only queries the effective process identity.
    let owner = unsafe { libc::geteuid() };
    let path = directory.path().join("value");
    fs::write(&path, b"private")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(/*mode*/ 0o600))?;
    let hard = directory.path().join("hard");
    fs::hard_link(&path, &hard)?;
    assert!(open_private_path(&path, owner, TrustedPathKind::File).is_err());
    fs::remove_file(&hard)?;
    let link = directory.path().join("link");
    symlink(&path, &link)?;
    assert!(open_private_path(&link, owner, TrustedPathKind::File).is_err());
    let alias = directory.path().join("alias");
    symlink(directory.path(), &alias)?;
    assert!(open_private_path(&alias.join("value"), owner, TrustedPathKind::File).is_err());
    open_private_path(&path, owner, TrustedPathKind::File)?;
    Ok(())
}

/// Classification follows the inode's filesystem and leaves unqualified kernel mounts unsupported.
#[test]
fn filesystem_probe_follows_open_inode() -> TestResult {
    let directory = directory()?;
    let file = File::create(directory.path().join("value"))?;
    assert_eq!(
        LinuxFilesystem::for_file(&file)?,
        LinuxFilesystem::for_file(&File::open(directory.path())?)?
    );
    assert_eq!(
        LinuxFilesystem::for_file(&File::open("/proc")?)?,
        LinuxFilesystem::Other
    );
    Ok(())
}
