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
pub(crate) mod storage;
use super::process::ModelProcessIsolation;
use crate::CheckoutHandOver;
use crate::hand_over::HandOverError;
use ora_logging::ora_warn;
use ora_node_protocol::{ExecutionId, GitIdentity};
use ora_plugin_lifecycle::{
    DenoPluginRuntimeLauncher, LaunchedRuntime, PluginLaunchRequest, PluginLogSetup,
    PluginRuntimeFailure, PluginRuntimeLauncher, PluginRuntimeTimeouts,
};
use ora_process::ProcessIdentity;
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
    /// clone workload's identity drop does — and tests keep `identity` inherited while
    /// naming their own ids, since only root may take another identity. `hand_over` gives the
    /// checkout to them under the Node-wide walk bound and host check.
    Separate {
        directory: PathBuf,
        identity: ProcessIdentity,
        uid: u32,
        gid: u32,
        hand_over: CheckoutHandOver,
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

/// Why a session's placement could not be prepared.
#[derive(Debug, thiserror::Error)]
pub(super) enum PlacementError {
    /// The checkout nests directories deeper than the hand-over walk descends, so it was not
    /// handed to the workload user and the session must not start in it. Every other hand-over
    /// failure, a host that stopped protecting hard links included, is [`Self::Io`].
    #[error(
        "checkout is nested deeper than {} directory levels",
        ora_utils::fs::MAX_OWN_TREE_DEPTH
    )]
    CheckoutTooDeep,
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl SessionPlacement {
    /// The attempt spawner applies the same identity to the plugin and its host children.
    pub(super) fn identity(&self) -> ProcessIdentity {
        match self {
            Self::Shared => ProcessIdentity::Inherit,
            Self::Separate { identity, .. } => *identity,
        }
    }

    /// A value-free key identifies the immutable placement to restart recovery.
    pub(super) fn view_key(&self) -> Option<String> {
        match self {
            Self::Shared => None,
            Self::Separate { directory, .. } => directory
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned),
        }
    }

    /// Once registered, an interrupted stack must leave views for the durable stop owner.
    pub(super) fn retain_for_stop(&mut self) {
        if let Self::Separate { directory, .. } = self {
            directory.remove_on_drop = false;
        }
    }

    /// Removes views only after affirmative stop proof; failed removal is retried by recovery.
    pub(super) fn release(&mut self) -> io::Result<()> {
        if let Self::Separate { directory, .. } = self {
            match fs::remove_dir_all(&directory.root) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            directory.remove_on_drop = false;
        }
        Ok(())
    }
    /// Prepares the session's directory and hands a checkout left by older Nodes, whose agents
    /// ran as root, back to the workload user. Runs blocking filesystem work, so callers keep it
    /// off the async workers.
    pub(super) fn prepare(
        workload: &SessionWorkload,
        execution: &ExecutionId,
        package_root: &Path,
        checkout: &Path,
    ) -> Result<Self, PlacementError> {
        match workload {
            SessionWorkload::Shared => Ok(Self::Shared),
            SessionWorkload::Separate {
                directory,
                identity,
                uid,
                gid,
                hand_over,
            } => {
                // Every session walks its checkout itself, before its own Agent starts, rather
                // than trusting an earlier sibling to have done it: admission order says nothing
                // about whether that sibling's walk finished, or ran at all. The walk pins every
                // entry before acting on it and never re-resolves a path, so live sibling Agents
                // changing the tree as the workload user cannot redirect it outside the checkout:
                // an entry removed before it is pinned is skipped and a name replaced afterwards
                // changes nothing the walk does. Hard links are only safe while the host keeps
                // `fs.protected_hardlinks`, which the hand-over reads again before this walk. An
                // already-right tree costs only a walk, and the depth bound plus the Node-wide
                // walk permits keep the descriptors all walks hold bounded.
                hand_over
                    .hand_over(checkout, *uid, *gid)
                    .map_err(|error| match error {
                        HandOverError::TooDeep => PlacementError::CheckoutTooDeep,
                        HandOverError::HardlinksUnprotected(error) | HandOverError::Io(error) => {
                            PlacementError::Io(error)
                        }
                    })?;
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
        mut runtime: std::collections::BTreeMap<String, String>,
        isolation: ModelProcessIsolation,
        spawner: super::stop::spawner::SessionAttemptSpawner,
    ) -> SessionLauncher {
        isolation.extend_environment(&mut runtime);
        let environment = SessionEnvironment::new(git_identity, runtime);
        match self {
            Self::Shared => PackageViewLauncher {
                inner: DenoPluginRuntimeLauncher::with_environment_provider(
                    PluginRuntimeTimeouts::default(),
                    environment,
                )
                .with_spawner(spawner),
                location: PackageLocation::Installed,
            },
            Self::Separate {
                directory,
                identity: _,
            } => PackageViewLauncher {
                inner: DenoPluginRuntimeLauncher::with_environment_provider(
                    PluginRuntimeTimeouts::default(),
                    environment.with_home(&directory.home),
                )
                .with_spawner(spawner),
                location: PackageLocation::View {
                    installed: package_root.to_path_buf(),
                    view: directory.package.clone(),
                },
            },
        }
    }

    /// Links immutable MCP package bytes beside the agent package for workload access. Plugin
    /// identities are hashed so their two segments cannot collide with the existing layout.
    pub(super) fn mcp_package(
        &self,
        id: &ora_domain::PluginId,
        installed: &Path,
    ) -> io::Result<PathBuf> {
        match self {
            Self::Shared => Ok(installed.to_path_buf()),
            Self::Separate { directory, .. } => {
                let view = directory.root.join(format!(
                    "mcp-{}",
                    ora_utils::hash::sha256_hex(
                        format!(
                            "{}:{}",
                            id.canonical(),
                            installed
                                .file_name()
                                .ok_or_else(|| io::Error::other("missing package version"))?
                                .to_string_lossy()
                        )
                        .as_bytes()
                    )
                ));
                match ora_utils::path::check_directory_without_symlinks(&view) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        ora_utils::fs::hard_link_tree_no_follow(installed, &view)?
                    }
                    Err(error) => return Err(error),
                }
                Ok(view)
            }
        }
    }

    /// Removes one obsolete immutable view only after every attempt referring to it stopped.
    pub(super) fn release_mcp_package(&self, id: &str, version: &str) -> io::Result<()> {
        if let Self::Separate { directory, .. } = self {
            let view = directory.root.join(format!(
                "mcp-{}",
                ora_utils::hash::sha256_hex(format!("{id}:{version}").as_bytes())
            ));
            match fs::remove_dir_all(view) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
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
    remove_on_drop: bool,
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
            remove_on_drop: true,
            package: root.join("package"),
            home: root.join("home"),
            root,
        };
        // From here on, dropping `directory` removes whatever a failure left behind.
        create_directory(&directory.root, /*mode*/ 0o711)?;
        ora_utils::fs::hard_link_tree_no_follow(package_root, &directory.package)?;
        create_directory(&directory.home, /*mode*/ 0o700)?;
        std::os::unix::fs::chown(&directory.home, Some(uid), Some(gid))?;
        Ok(directory)
    }
}

impl Drop for SessionDirectory {
    /// Runs once the plugin stopped (or never started): nothing reads the view or the home
    /// anymore. A failure is only logged; the next service start purges the directory anyway.
    fn drop(&mut self) {
        if !self.remove_on_drop {
            return;
        }
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
