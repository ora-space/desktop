//! Maps each plugin kind to the Deno sandbox flags its process is launched with.
//!
//! This module is the single source of truth for plugin permissions: the lifecycle uses it for
//! packages it activates, and the backend's agent connection supervisor uses the same agent set,
//! so the two launch paths cannot drift apart.
//!
//! Permissions are not how a plugin gains capabilities. A workbench plugin runs with no grants
//! at all and reaches its data through the `ora/storage/*` host methods; the flags below exist
//! only for the agent kind, whose own CLI still needs the host. A webview, skill, or MCP plugin
//! has no process and is never launched.

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
    /// Grants `--allow-env` for the whole process environment.
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
/// A workbench plugin gets nothing: under `--no-prompt` every filesystem, network, and
/// environment access is a hard `PermissionDenied`, and everything it legitimately needs (its
/// data directory) is served by the host over `ora/storage/*`. An agent plugin keeps the broad
/// grants it has always had (see `agent_permissions`); narrowing them is deliberately out of
/// scope here. Webview, skill, and MCP plugins are never launched, so their empty sets only make
/// the match exhaustive.
pub fn permissions_for(contribution: &PluginContribution) -> Vec<DenoPermission> {
    match contribution {
        PluginContribution::Agent(_) => agent_permissions(),
        PluginContribution::Workbench(_)
        | PluginContribution::Webview(_)
        | PluginContribution::Skill(_)
        | PluginContribution::Mcp(_) => Vec::new(),
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
        InstalledPluginAgent, InstalledWorkbenchDescriptor, PluginContribution,
    };
    use ora_utils::path::PortableRelativePath;
    use pretty_assertions::assert_eq;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Builds one workbench contribution with a static page.
    fn workbench_contribution() -> PluginContribution {
        PluginContribution::Workbench(InstalledWorkbenchDescriptor {
            entrypoint: PortableRelativePath::parse("main.js").expect("entrypoint"),
            asset_root: PathBuf::from("/plugins/example/assets"),
            page_entry: PortableRelativePath::parse("index.html").expect("page entry"),
            declared_methods: Vec::new(),
        })
    }

    /// Agent plugins keep the historical unscoped grants.
    #[test]
    fn agent_plugins_get_broad_permissions() {
        let contribution = PluginContribution::Agent(InstalledPluginAgent {
            display_name: "Agent".to_string(),
            entrypoint: PortableRelativePath::parse("main.js").expect("entrypoint"),
        });
        let permissions = permissions_for(&contribution);
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

    /// Workbench plugins launch with no permission flags at all; their data goes through the host.
    #[test]
    fn workbench_plugins_get_no_permissions() {
        assert_eq!(permissions_for(&workbench_contribution()), Vec::new());
    }

    /// Skill plugins are static packages and never receive runtime permissions.
    #[test]
    fn skill_plugins_get_no_permissions() {
        assert_eq!(
            permissions_for(&PluginContribution::Skill(Default::default())),
            Vec::new()
        );
    }

    /// MCP plugins are configuration-only and never receive Deno permissions.
    #[test]
    fn mcp_plugins_get_no_permissions() {
        use ora_plugin_config::{CompiledConfigurationFile, compile_configuration_file};
        use ora_plugin_manager::InstalledMcpDescriptor;

        let CompiledConfigurationFile::Mcp(configuration) = compile_configuration_file(
            br#"{ "schemaVersion": 1, "transport": { "type": "http", "url": "https://mcp.example.com/v1" } }"#,
        )
        .expect("compile fixture") else {
            panic!("expected the MCP shape");
        };
        assert_eq!(
            permissions_for(&PluginContribution::Mcp(InstalledMcpDescriptor {
                configuration
            })),
            Vec::new()
        );
    }

    /// Scoped grants render the canonical path so Deno's own canonical comparison matches.
    #[test]
    fn scoped_grants_render_canonical_paths() {
        let temp_dir = TempDir::new().expect("create data directory");
        let data_dir = temp_dir.path().join("scoped");
        std::fs::create_dir_all(&data_dir).expect("create scoped directory");
        let canonical = std::fs::canonicalize(&data_dir).expect("canonicalize scoped directory");

        let mut read = OsString::from("--allow-read=");
        read.push(&canonical);
        let mut write = OsString::from("--allow-write=");
        write.push(&canonical);
        assert_eq!(
            (
                DenoPermission::AllowRead(ReadScope::Directory(data_dir.clone())).to_flag(),
                DenoPermission::AllowWrite(data_dir).to_flag(),
            ),
            (Ok(read), Ok(write)),
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
