//! Where one session's agent runs when the deployment separates it from the Node identity.
//!
//! A sandbox Node manages as root with an owner-private umask, so its data directory — installed
//! packages included — is unreadable to the workload user, while the checkout belongs to that
//! user. Running the agent as root would let it read Node secrets and make Git refuse the
//! checkout as dubiously owned; running it as the workload user needs a package it can read and
//! a home it can write. Each session therefore gets its own directory under the deployment's
//! workload directory:
//!
//! - `package/`: a hard-link view of the installed package, in root-owned `0755` directories, so
//!   the workload user reads exactly the installed files and can change none of them;
//! - `home/`: `0700` and owned by the workload user, holding everything the agent and its tools
//!   write for themselves (Deno cache, CLI configuration and state).
//!
//! The session directory itself is root-owned `0711`: the workload user can reach both children
//! by name but cannot list or replace them. It lives exactly as long as the session's plugin.

use super::host::{SessionEnvironment, SessionLauncher};
use ora_logging::ora_warn;
use ora_node_protocol::{ExecutionId, GitIdentity};
use ora_plugin_lifecycle::{
    DenoPluginRuntimeLauncher, LaunchedRuntime, PluginLaunchRequest, PluginLogSetup,
    PluginRuntimeFailure, PluginRuntimeLauncher, PluginRuntimeTimeouts,
};
use ora_process::{ProcessIdentity, TokioProcessSpawner};
use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// How a Node runs its agents relative to its own OS identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionWorkload {
    /// Agents run as the Node itself, from the installed package, with the Node's home.
    Shared,
    /// Agents run as the workload user from per-session directories under `directory`.
    ///
    /// `identity` is what processes are spawned as; `uid` and `gid` own the session home and the
    /// checkout. Production names the same user in both — a group equal to the user, as the
    /// clone workload's `setpriv --regid` does — and tests keep `identity` inherited while
    /// naming their own ids, since only root may take another identity.
    Separate {
        directory: PathBuf,
        identity: ProcessIdentity,
        uid: u32,
        gid: u32,
    },
}

/// Where one session's plugin runs from, and what has to be removed once it stopped.
pub(super) enum SessionPlacement {
    Shared,
    Separate {
        directory: SessionDirectory,
        identity: ProcessIdentity,
    },
}

impl SessionPlacement {
    /// Prepares the session's directory and hands a checkout left by older Nodes, whose agents
    /// ran as root, back to the workload user. Runs blocking filesystem work, so callers keep it
    /// off the async workers.
    pub(super) fn prepare(
        workload: &SessionWorkload,
        execution: &ExecutionId,
        package_root: &Path,
        checkout: &Path,
    ) -> io::Result<Self> {
        match workload {
            SessionWorkload::Shared => Ok(Self::Shared),
            SessionWorkload::Separate {
                directory,
                identity,
                uid,
                gid,
            } => {
                // A link the old agent left cannot redirect this privileged change: nothing is
                // followed, and an already-right tree costs only a walk.
                ora_utils::fs::own_tree_no_follow(checkout, *uid, *gid)?;
                Ok(Self::Separate {
                    directory: SessionDirectory::create(
                        directory,
                        execution,
                        package_root,
                        *uid,
                        *gid,
                    )?,
                    identity: *identity,
                })
            }
        }
    }

    /// Composes the plugin launcher for this placement: the session's commit identity always,
    /// and in a separate placement the session home, the package view and the workload identity.
    pub(super) fn launcher(
        &self,
        git_identity: &GitIdentity,
        package_root: &Path,
        runtime: std::collections::BTreeMap<String, String>,
    ) -> SessionLauncher {
        let environment = SessionEnvironment::new(git_identity, runtime);
        match self {
            Self::Shared => PackageViewLauncher {
                inner: DenoPluginRuntimeLauncher::with_environment_provider(
                    PluginRuntimeTimeouts::default(),
                    environment,
                ),
                location: PackageLocation::Installed,
            },
            Self::Separate {
                directory,
                identity,
            } => PackageViewLauncher {
                inner: DenoPluginRuntimeLauncher::with_environment_provider(
                    PluginRuntimeTimeouts::default(),
                    environment.with_home(&directory.home),
                )
                .with_spawner(TokioProcessSpawner::running_as(*identity)),
                location: PackageLocation::View {
                    installed: package_root.to_path_buf(),
                    view: directory.package.clone(),
                },
            },
        }
    }

    /// The private workload home owns CLI state; management-owned Node directories remain closed.
    pub(super) fn home_directory(&self) -> Option<&Path> {
        match self {
            Self::Shared => None,
            Self::Separate { directory, .. } => Some(&directory.home),
        }
    }

    /// Publishes only the model CA beside the package view, outside the workload-writable home.
    pub(super) fn publish_model_ca(
        &self,
        access: &mut crate::model_proxy::ModelAccess,
    ) -> Result<(), crate::model_proxy::ModelAccessError> {
        match self {
            Self::Shared => Ok(()),
            Self::Separate { directory, .. } => {
                access.publish_ca(&directory.root.join("model-ca.pem"))
            }
        }
    }
}

/// One session's directory under the workload directory, removed when dropped.
pub(super) struct SessionDirectory {
    root: PathBuf,
    package: PathBuf,
    home: PathBuf,
}

impl SessionDirectory {
    /// Builds the session directory from scratch; whatever an earlier attempt left under the same
    /// name predates this session and is discarded rather than reused.
    fn create(
        workload_directory: &Path,
        execution: &ExecutionId,
        package_root: &Path,
        uid: u32,
        gid: u32,
    ) -> io::Result<Self> {
        // Execution IDs are opaque protocol text; a digest keeps the name one safe component,
        // like the Revision delivery directories.
        let root =
            workload_directory.join(ora_utils::hash::sha256_hex(execution.as_str().as_bytes()));
        match fs::remove_dir_all(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let directory = Self {
            package: root.join("package"),
            home: root.join("home"),
            root,
        };
        // From here on, dropping `directory` removes whatever a failure left behind.
        create_directory(&directory.root, /*mode*/ 0o711)?;
        ora_utils::fs::link_tree_no_follow(package_root, &directory.package)?;
        create_directory(&directory.home, /*mode*/ 0o700)?;
        std::os::unix::fs::chown(&directory.home, Some(uid), Some(gid))?;
        Ok(directory)
    }
}

impl Drop for SessionDirectory {
    /// Runs once the plugin stopped (or never started): nothing reads the view or the home
    /// anymore. A failure is only logged; the next service start purges the directory anyway.
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.root) {
            ora_warn!(path = %self.root.display(), error = %error, "session workload directory was not removed");
        }
    }
}

/// Creates one directory with exactly `mode`, which the creation mode alone would leave to the
/// umask.
fn create_directory(path: &Path, mode: u32) -> io::Result<()> {
    fs::DirBuilder::new().mode(mode).create(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// Removes every session directory a previous Node process left, returning how many it removed.
///
/// No session survives a Node restart, so nothing under the workload directory is in use when
/// the service opens. Entries are removed without following links.
pub(crate) fn purge_workload_directory(workload_directory: &Path) -> io::Result<usize> {
    let mut removed = 0;
    for entry in fs::read_dir(workload_directory)? {
        let path = entry?.path();
        if fs::symlink_metadata(&path)?.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
        removed += 1;
    }
    Ok(removed)
}

/// Where the launched plugin's package is read from.
#[derive(Clone, Debug)]
enum PackageLocation {
    /// Straight from the installed package.
    Installed,
    /// From `view`, a copy of the package installed at `installed`.
    View { installed: PathBuf, view: PathBuf },
}

/// Launches the session's plugin from its package view instead of the installed package.
///
/// The lifecycle keeps discovering and verifying the installed package; only the launch request
/// is moved, so the process's working directory, entrypoint and `packageCommand` root all lie in
/// the view the workload user can read.
#[derive(Clone, Debug)]
pub(super) struct PackageViewLauncher<L> {
    inner: L,
    location: PackageLocation,
}

impl<L: PluginRuntimeLauncher> PluginRuntimeLauncher for PackageViewLauncher<L> {
    type Runtime = L::Runtime;

    /// Refuses to launch any package but the one the view mirrors: the session lifecycle runs
    /// exactly one plugin, so another root would mean launching something the view does not hold.
    fn launch(
        &self,
        request: PluginLaunchRequest,
        log: PluginLogSetup,
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send
    {
        let request = match &self.location {
            PackageLocation::Installed => Ok(request),
            PackageLocation::View { installed, view } => in_view(request, installed, view),
        };
        let inner = self.inner.clone();
        async move { inner.launch(request?, log).await }
    }
}

/// Moves a launch request from the installed package into its view.
fn in_view(
    mut request: PluginLaunchRequest,
    installed: &Path,
    view: &Path,
) -> Result<PluginLaunchRequest, PluginRuntimeFailure> {
    // Discovery and the catalog may spell the same directory differently.
    let same_package = match (
        request.package_root.canonicalize(),
        installed.canonicalize(),
    ) {
        (Ok(requested), Ok(installed)) => requested == installed,
        (Err(_), _) | (_, Err(_)) => false,
    };
    let entrypoint = request
        .entrypoint
        .strip_prefix(&request.package_root)
        .map(|relative| view.join(relative));
    match (same_package, entrypoint) {
        (true, Ok(entrypoint)) => {
            request.entrypoint = entrypoint;
            request.package_root = view.to_path_buf();
            Ok(request)
        }
        (false, _) | (_, Err(_)) => Err(PluginRuntimeFailure::new(
            "the launched package is not the session's package view",
        )),
    }
}

#[cfg(test)]
mod tests;
