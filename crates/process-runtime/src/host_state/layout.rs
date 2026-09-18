use std::fs::{self, DirBuilder, File, OpenOptions};
use std::os::unix::{
    ffi::OsStrExt,
    fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
};
use std::path::{Path, PathBuf};

use ora_process_protocol::ScopeId;
use ora_utils::{
    fs::LinuxFilesystem,
    path::{TrustedPathKind, open_private_path, open_trusted_path},
};

use super::ProcessStateError;

/// Owns path policy; only the journal owner decides which durable layout names are recognized.
pub(super) struct HostLayout {
    path: PathBuf,
    directory: File,
    owner: u32,
}

impl HostLayout {
    /// Creates exactly one dedicated directory; no existing directory or parent is repaired.
    pub(super) fn create(path: &Path) -> Result<Self, ProcessStateError> {
        check_endpoint_length(path)?;
        // SAFETY: geteuid has no preconditions and does not modify process identity.
        let owner = unsafe { libc::geteuid() };
        let parent = path
            .parent()
            .ok_or(ProcessStateError::Rejected("missing state parent"))?;
        let parent = open_trusted_path(parent, owner, TrustedPathKind::Directory)?;
        require_local_filesystem(&parent)?;
        DirBuilder::new().mode(/*mode*/ 0o700).create(path)?;
        parent.sync_all()?;
        let layout = Self::open(path)?;
        DirBuilder::new()
            .mode(/*mode*/ 0o700)
            .create(path.join("scopes"))?;
        layout.sync()?;
        Ok(layout)
    }

    /// Opens only private local state; links and untrusted ancestors fail before journal access.
    pub(super) fn open(path: &Path) -> Result<Self, ProcessStateError> {
        check_endpoint_length(path)?;
        // SAFETY: geteuid only queries the current effective identity.
        let owner = unsafe { libc::geteuid() };
        let directory = open_private_path(path, owner, TrustedPathKind::Directory)?;
        require_local_filesystem(&directory)?;
        Ok(Self {
            path: path.to_owned(),
            directory,
            owner,
        })
    }

    /// Creates a private regular file exclusively, never truncating an existing entry.
    pub(super) fn create_file(&self, name: &str) -> Result<File, ProcessStateError> {
        let file = OpenOptions::new()
            .read(/*read*/ true)
            .write(/*write*/ true)
            .create_new(/*create_new*/ true)
            .mode(/*mode*/ 0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(self.path.join(name))?;
        file.sync_all()?;
        Ok(file)
    }

    /// Pins a recognized private file without following links or granting creation permission.
    pub(super) fn open_file(&self, name: &str) -> Result<File, ProcessStateError> {
        Ok(open_private_path(
            &self.path.join(name),
            self.owner,
            TrustedPathKind::File,
        )?)
    }

    /// Rejects unknown entries and missing required files; SQLite owns only its known sidecars.
    pub(super) fn validate_entries(&self) -> Result<Vec<ScopeId>, ProcessStateError> {
        self.open_file("host.sqlite")?;
        let scopes = open_private_path(
            &self.path.join("scopes"),
            self.owner,
            TrustedPathKind::Directory,
        )?;
        require_local_filesystem(&scopes)?;
        for entry in fs::read_dir(&self.path)? {
            let entry = entry?;
            match entry.file_name().to_str() {
                Some("host.lock" | "host.sqlite" | "host.sqlite-wal" | "host.sqlite-shm") => {
                    self.open_file(
                        entry
                            .file_name()
                            .to_str()
                            .ok_or(ProcessStateError::Rejected("invalid layout name"))?,
                    )?;
                }
                Some("scopes") => {}
                Some("host.sock" | "host-io.sock") => {
                    self.validate_socket(&entry.path())?;
                }
                _ => return Err(ProcessStateError::Rejected("unknown state directory entry")),
            }
        }
        let mut scopes = Vec::new();
        for entry in fs::read_dir(self.path.join("scopes"))? {
            let entry = entry?;
            let scope = entry
                .file_name()
                .to_str()
                .ok_or(ProcessStateError::Rejected("invalid scope directory name"))?
                .parse()?;
            open_private_path(&entry.path(), self.owner, TrustedPathKind::Directory)?;
            scopes.push(scope);
        }
        // Scope contents belong to guardians, not the host. The bootstrap layer must validate
        // them before doing anything beyond reading the original host creation responsibility.
        Ok(scopes)
    }

    /// Prevents registering a fresh intent over an unaccounted-for scope directory or link.
    pub(super) fn reject_existing_scope(&self, scope: ScopeId) -> Result<(), ProcessStateError> {
        open_private_path(
            &self.path.join("scopes"),
            self.owner,
            TrustedPathKind::Directory,
        )?;
        let path = self.path.join("scopes").join(scope.to_string());
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
            Ok(_) => Err(ProcessStateError::Rejected(
                "scope path already exists without a creation intent",
            )),
        }
    }

    /// Keeps the dedicated host database location out of caller-selected filenames.
    pub(super) fn database_path(&self) -> PathBuf {
        self.path.join("host.sqlite")
    }

    /// Locates one canonical scope without accepting caller-controlled path fragments.
    pub(super) fn scope_path(&self, scope: ScopeId) -> PathBuf {
        self.path.join("scopes").join(scope.to_string())
    }

    /// Flushes directory entries after journal commits that may create SQLite sidecars.
    pub(super) fn sync(&self) -> Result<(), ProcessStateError> {
        self.directory.sync_all()?;
        Ok(())
    }

    /// Only a private socket can occupy a recognized endpoint name; other user files are preserved.
    fn validate_socket(&self, path: &Path) -> Result<(), ProcessStateError> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != self.owner
            || metadata.mode() & 0o777 != 0o600
            || metadata.nlink() != 1
        {
            return Err(ProcessStateError::Rejected("invalid host endpoint inode"));
        }
        Ok(())
    }

    /// Replaces only a refused private socket under the already-held stable host lock.
    pub(super) async fn bind_endpoint(
        &self,
        name: &str,
    ) -> Result<tokio::net::UnixListener, ProcessStateError> {
        let path = self.path.join(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                self.validate_socket(&path)?;
                match tokio::time::timeout(
                    std::time::Duration::from_secs(/*secs*/ 1),
                    tokio::net::UnixStream::connect(&path),
                )
                .await
                {
                    Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => {}
                    _ => {
                        return Err(ProcessStateError::Rejected(
                            "host endpoint may still be active",
                        ));
                    }
                }
                fs::remove_file(&path)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let listener = tokio::net::UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(/*mode*/ 0o600))?;
        self.sync()?;
        Ok(listener)
    }
}

/// Reserves the full canonical Scope/socket suffix rather than truncating long Unix socket paths.
fn check_endpoint_length(path: &Path) -> Result<(), ProcessStateError> {
    let endpoint = path
        .join("scopes")
        .join("00000000-0000-0000-0000-000000000001")
        .join("control.sock");
    if !path.is_absolute() || endpoint.as_os_str().as_bytes().len() >= 108 {
        return Err(ProcessStateError::Rejected(
            "state path must be absolute and fit Linux Unix socket paths",
        ));
    }
    Ok(())
}

/// Limits the first deployment set; this is a filesystem gate, not a hardware durability proof.
fn require_local_filesystem(file: &File) -> Result<(), ProcessStateError> {
    match LinuxFilesystem::for_file(file)? {
        LinuxFilesystem::Ext | LinuxFilesystem::Xfs | LinuxFilesystem::Btrfs => Ok(()),
        LinuxFilesystem::Other => Err(ProcessStateError::Rejected(
            "unsupported process state filesystem",
        )),
    }
}
