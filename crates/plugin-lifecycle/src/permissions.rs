//! Maps each plugin kind to the Deno sandbox flags its process is launched with.
//!
//! This module is the single source of truth for plugin permissions: the lifecycle uses it for
//! packages it activates, and the backend's agent connection supervisor uses the same agent set,
//! so the two launch paths cannot drift apart.

use ora_plugin_manager::PluginContribution;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Names the directory a read grant covers, keeping an unscoped grant explicit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadScope {
    /// Grants `--allow-read` without a path, i.e. the whole filesystem.
    Everything,
    /// Grants read access to one directory tree only.
    Directory(PathBuf),
}

/// One Deno permission flag placed before the plugin entrypoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenoPermission {
    AllowRead(ReadScope),
    AllowWrite(PathBuf),
    AllowRun,
    AllowEnv,
    AllowNet,
}

/// Reports why a permission cannot be rendered into a launchable flag.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PermissionFlagError {
    #[error("permission path `{path}` could not be canonicalized: {reason}")]
    Canonicalize { path: PathBuf, reason: String },
    #[error("permission path `{0}` contains a comma, which Deno reads as a list separator")]
    CommaInPath(PathBuf),
}

impl DenoPermission {
    /// Renders the permission as the exact `--allow-*` flag Deno expects.
    ///
    /// Paths are canonicalized first so the grant matches what Deno resolves at runtime (Deno
    /// compares canonical paths, and a symlinked data directory would otherwise be denied). A
    /// comma inside a path is rejected rather than escaped because Deno has no escape syntax:
    /// it would silently split the grant into two unintended ones.
    pub fn to_flag(&self) -> Result<OsString, PermissionFlagError> {
        match self {
            Self::AllowRead(ReadScope::Everything) => Ok(OsString::from("--allow-read")),
            Self::AllowRead(ReadScope::Directory(path)) => scoped_flag("--allow-read", path),
            Self::AllowWrite(path) => scoped_flag("--allow-write", path),
            Self::AllowRun => Ok(OsString::from("--allow-run")),
            Self::AllowEnv => Ok(OsString::from("--allow-env")),
            Self::AllowNet => Ok(OsString::from("--allow-net")),
        }
    }
}

/// Builds `<flag>=<canonical path>` after validating the path is representable as one grant.
fn scoped_flag(flag: &str, path: &Path) -> Result<OsString, PermissionFlagError> {
    let canonical =
        std::fs::canonicalize(path).map_err(|error| PermissionFlagError::Canonicalize {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;
    if canonical.as_os_str().as_encoded_bytes().contains(&b',') {
        return Err(PermissionFlagError::CommaInPath(canonical));
    }
    let mut rendered = OsString::from(flag);
    rendered.push("=");
    rendered.push(canonical);
    Ok(rendered)
}

/// Returns the permission set granted to every plugin of the contribution's kind.
///
/// `data_dir` is the plugin's private data directory; it is the only place a ui plugin may
/// touch, while an agent plugin keeps the broad grants it has always had (see
/// `agent_permissions`). Narrowing agent permissions is deliberately out of scope here.
pub fn permissions_for(contribution: &PluginContribution, data_dir: &Path) -> Vec<DenoPermission> {
    match contribution {
        PluginContribution::Agent(_) => agent_permissions(),
        PluginContribution::Ui(_) => vec![
            DenoPermission::AllowRead(ReadScope::Directory(data_dir.to_path_buf())),
            DenoPermission::AllowWrite(data_dir.to_path_buf()),
        ],
    }
}

/// Returns the broad permission set an agent plugin needs to spawn and drive its agent CLI.
///
/// An agent plugin owns the agent process itself, so it needs `--allow-run` plus whatever that
/// CLI needs. That makes it roughly as privileged as the host: a deliberate, documented gap that
/// closes later by changing only how the agent is started, never the `agent/acp` pipe.
pub fn agent_permissions() -> Vec<DenoPermission> {
    vec![
        DenoPermission::AllowRun,
        DenoPermission::AllowRead(ReadScope::Everything),
        DenoPermission::AllowEnv,
        DenoPermission::AllowNet,
    ]
}

#[cfg(test)]
mod tests {
    use super::{DenoPermission, PermissionFlagError, ReadScope, permissions_for};
    use ora_plugin_manager::{
        InstalledPluginAgent, InstalledPluginUi, InstalledSurface, InstalledSurfaceSource,
        InstancePolicy, PluginContribution, RemoteSiteSource, SurfaceId, WebDataPolicy,
    };
    use pretty_assertions::assert_eq;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Builds one ui contribution with a single remote-site surface.
    fn ui_contribution() -> PluginContribution {
        PluginContribution::Ui(InstalledPluginUi {
            contract_version: 1,
            surfaces: vec![InstalledSurface {
                id: SurfaceId::parse("market").expect("surface id"),
                title: "Market".to_string(),
                instance_policy: InstancePolicy::Singleton,
                source: InstalledSurfaceSource::RemoteSite(RemoteSiteSource {
                    entry_url: "https://example.com/".parse().expect("entry url"),
                    allow_hosts: Vec::new(),
                    allow_host_suffixes: Vec::new(),
                    web_data: WebDataPolicy::PersistentProfile,
                }),
            }],
        })
    }

    /// Agent plugins keep the historical unscoped grants.
    #[test]
    fn agent_plugins_get_broad_permissions() {
        let contribution = PluginContribution::Agent(InstalledPluginAgent {
            display_name: "Agent".to_string(),
            contract_version: 1,
        });
        let permissions = permissions_for(&contribution, &PathBuf::from("/unused"));
        let flags = permissions
            .iter()
            .map(|permission| permission.to_flag().expect("render agent flag"))
            .collect::<Vec<_>>();

        assert_eq!(
            (permissions, flags),
            (
                vec![
                    DenoPermission::AllowRun,
                    DenoPermission::AllowRead(ReadScope::Everything),
                    DenoPermission::AllowEnv,
                    DenoPermission::AllowNet,
                ],
                vec![
                    OsString::from("--allow-run"),
                    OsString::from("--allow-read"),
                    OsString::from("--allow-env"),
                    OsString::from("--allow-net"),
                ],
            ),
        );
    }

    /// Ui plugins may only read and write their own data directory, rendered canonically.
    #[test]
    fn ui_plugins_are_scoped_to_their_data_directory() {
        let temp_dir = TempDir::new().expect("create data directory");
        let data_dir = temp_dir.path().join("plugin-data").join("ora.example");
        std::fs::create_dir_all(&data_dir).expect("create plugin data directory");
        let canonical = std::fs::canonicalize(&data_dir).expect("canonicalize data directory");

        let permissions = permissions_for(&ui_contribution(), &data_dir);
        let flags = permissions
            .iter()
            .map(|permission| permission.to_flag().expect("render ui flag"))
            .collect::<Vec<_>>();

        let mut read = OsString::from("--allow-read=");
        read.push(&canonical);
        let mut write = OsString::from("--allow-write=");
        write.push(&canonical);
        assert_eq!(
            (permissions, flags),
            (
                vec![
                    DenoPermission::AllowRead(ReadScope::Directory(data_dir.clone())),
                    DenoPermission::AllowWrite(data_dir),
                ],
                vec![read, write],
            ),
        );
    }

    /// A comma in a granted path would be read as two grants, so it refuses to render.
    #[test]
    fn rejects_paths_containing_a_comma() {
        let temp_dir = TempDir::new().expect("create data directory");
        let data_dir = temp_dir.path().join("a,b");
        std::fs::create_dir_all(&data_dir).expect("create comma directory");
        let canonical = std::fs::canonicalize(&data_dir).expect("canonicalize comma directory");

        assert_eq!(
            DenoPermission::AllowWrite(data_dir).to_flag(),
            Err(PermissionFlagError::CommaInPath(canonical)),
        );
    }

    /// A missing path cannot be canonicalized and therefore cannot be granted.
    #[test]
    fn rejects_paths_that_do_not_exist() {
        let temp_dir = TempDir::new().expect("create data directory");
        let missing = temp_dir.path().join("missing");

        let error = DenoPermission::AllowRead(ReadScope::Directory(missing.clone()))
            .to_flag()
            .unwrap_err();
        assert!(
            matches!(&error, PermissionFlagError::Canonicalize { path, .. } if *path == missing),
            "unexpected error: {error:?}"
        );
    }
}
