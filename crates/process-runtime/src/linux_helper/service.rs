use std::future::Future;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use ora_utils::path::{TrustedPathKind, open_trusted_path};
use tokio::net::UnixListener;

use super::load_deployment;

/// Starts an inspection-only endpoint after validating administrator-controlled deployment.
/// Existing paths are never removed to make room for a listener, including stale sockets.
pub async fn serve_linux_helper(
    config_path: &Path,
    socket_path: &Path,
    shutdown: impl Future<Output = ()>,
) -> Result<(), String> {
    let config = load_deployment(config_path)?;
    config.verify_deployment()?;
    let parent = socket_path
        .parent()
        .ok_or("socket needs an absolute parent")?;
    // A root-controlled parent prevents either manager or workload from replacing this endpoint.
    let _directory = open_trusted_path(parent, /*owner*/ 0, TrustedPathKind::Directory)
        .map_err(|error| error.to_string())?;
    if socket_path.file_name().is_none() {
        return Err("socket needs a file name".into());
    }
    let listener = UnixListener::bind(socket_path).map_err(|error| error.to_string())?;
    let metadata = std::fs::symlink_metadata(socket_path).map_err(|error| error.to_string())?;
    let _endpoint = OwnedEndpoint {
        path: socket_path.to_owned(),
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    // No connection is accepted until access is restricted. SO_PEERCRED also rejects any
    // unauthorized connection queued during bind, regardless of the creating process's umask.
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(/*mode*/ 0o600))
        .map_err(|error| error.to_string())?;
    std::os::unix::fs::chown(socket_path, Some(config.0.manager_uid), /*gid*/ None)
        .map_err(|error| error.to_string())?;
    config
        .serve_management(listener, shutdown)
        .await
        .map_err(|error| error.to_string())
}

struct OwnedEndpoint {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl Drop for OwnedEndpoint {
    /// Normal shutdown only removes the inode this service created; crash leftovers fail closed.
    fn drop(&mut self) {
        if let Ok(metadata) = std::fs::symlink_metadata(&self.path)
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
