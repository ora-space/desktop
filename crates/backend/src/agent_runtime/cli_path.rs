use super::runtime_internal;
use crate::BackendError;
use ora_domain::AgentCli;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Resolves one agent CLI executable with the same semantics on every platform.
///
/// Candidates are checked in order: each directory on the provided `PATH`
/// value first, then the CLI's fixed per-user install directory
/// (`~/.{cli}/bin`). PATH wins deliberately so the result matches what
/// `which` reports in the user's terminal, while the fixed directory keeps
/// official install-script setups working when the app inherits the minimal
/// PATH of a desktop-launched GUI process.
///
/// `path_variable` is injected instead of read here so tests can drive the
/// lookup without mutating the process environment.
pub(super) fn resolve_agent_cli_path(
    agent_cli: AgentCli,
    path_variable: Option<&OsStr>,
    home_directory: &Path,
) -> Result<PathBuf, BackendError> {
    let executable_name = agent_cli.executable_name();
    let path_directories: Vec<PathBuf> = path_variable
        .map(|value| std::env::split_paths(value).collect())
        .unwrap_or_default();
    for directory in &path_directories {
        if let Some(found) = find_in_directory(directory, executable_name) {
            return Ok(found);
        }
    }
    // Each CLI's official install script places the binary under this fixed
    // per-user directory, which is often absent from a GUI process PATH.
    let install_directory = match agent_cli {
        AgentCli::OpenCode => ".opencode",
        AgentCli::Nga => ".nga",
        AgentCli::CodeAgentCli => ".codeagentcli",
        AgentCli::Claude => ".claude",
        AgentCli::Codex => ".codex",
    };
    let fallback_directory = home_directory.join(install_directory).join("bin");
    if let Some(found) = find_in_directory(&fallback_directory, executable_name) {
        return Ok(found);
    }
    // Enumerate every searched source so "installed but not found" reports are
    // diagnosable from the log alone.
    Err(runtime_internal(
        "agent_cli_not_found",
        format!(
            "{executable_name} executable not found; searched PATH ({count} directories) and {fallback}",
            count = path_directories.len(),
            fallback = fallback_directory.display(),
        ),
    ))
}

/// Returns the executable candidate in `directory` when it is a runnable regular file.
///
/// Symlinks are followed on purpose: npm and bun install global binaries as
/// symlinks into their bin directories, so the type and permission checks must
/// describe the link target.
#[cfg(unix)]
fn find_in_directory(directory: &Path, executable_name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let candidate = directory.join(executable_name);
    let metadata = std::fs::metadata(&candidate).ok()?;
    (metadata.is_file() && metadata.permissions().mode() & 0o111 != 0).then_some(candidate)
}

/// Returns the executable candidate in `directory` for any `PATHEXT` extension.
///
/// Windows resolves executables by extension, so this mirrors `where.exe`:
/// npm global installs are `.cmd` shims, which the default list covers.
#[cfg(windows)]
fn find_in_directory(directory: &Path, executable_name: &str) -> Option<PathBuf> {
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string());
    pathext
        .split(';')
        .filter(|extension| !extension.is_empty())
        .map(|extension| directory.join(format!("{executable_name}{extension}")))
        .find(|candidate| candidate.is_file())
}

#[cfg(all(test, unix))]
mod tests {
    use super::resolve_agent_cli_path;
    use crate::ErrorClassification;
    use ora_contracts::{EmptyErrorParams, PublicError};
    use ora_domain::AgentCli;
    use pretty_assertions::assert_eq;
    use std::ffi::OsString;
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    /// Creates an executable file so a directory counts as providing the CLI.
    fn write_executable(directory: &Path, name: &str) -> std::path::PathBuf {
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Joins directories into a PATH-style value without touching the process environment.
    fn path_value(directories: &[&Path]) -> OsString {
        std::env::join_paths(directories.iter()).unwrap()
    }

    /// A PATH hit must win over the fixed install directory.
    #[test]
    fn prefers_path_over_fixed_install_directory() {
        let home = tempfile::tempdir().unwrap();
        let path_dir = tempfile::tempdir().unwrap();
        let on_path = write_executable(path_dir.path(), "opencode");
        write_executable(&home.path().join(".opencode").join("bin"), "opencode");

        let resolved = resolve_agent_cli_path(
            AgentCli::OpenCode,
            Some(&path_value(&[path_dir.path()])),
            home.path(),
        )
        .unwrap();

        assert_eq!(resolved, on_path);
    }

    /// The official install directory keeps working when PATH misses the CLI.
    #[test]
    fn falls_back_to_fixed_install_directory() {
        let home = tempfile::tempdir().unwrap();
        let empty_path_dir = tempfile::tempdir().unwrap();
        let installed = write_executable(&home.path().join(".opencode").join("bin"), "opencode");

        let resolved = resolve_agent_cli_path(
            AgentCli::OpenCode,
            Some(&path_value(&[empty_path_dir.path()])),
            home.path(),
        )
        .unwrap();

        assert_eq!(resolved, installed);
    }

    /// Earlier PATH directories win, matching shell `which` semantics.
    #[test]
    fn respects_path_directory_order() {
        let home = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let expected = write_executable(first.path(), "opencode");
        write_executable(second.path(), "opencode");

        let resolved = resolve_agent_cli_path(
            AgentCli::OpenCode,
            Some(&path_value(&[first.path(), second.path()])),
            home.path(),
        )
        .unwrap();

        assert_eq!(resolved, expected);
    }

    /// A same-named file without any execute bit must not satisfy the lookup.
    #[test]
    fn skips_non_executable_files() {
        let home = tempfile::tempdir().unwrap();
        let path_dir = tempfile::tempdir().unwrap();
        let plain = path_dir.path().join("opencode");
        std::fs::write(&plain, "not a binary").unwrap();
        std::fs::set_permissions(&plain, Permissions::from_mode(0o644)).unwrap();
        let installed = write_executable(&home.path().join(".opencode").join("bin"), "opencode");

        let resolved = resolve_agent_cli_path(
            AgentCli::OpenCode,
            Some(&path_value(&[path_dir.path()])),
            home.path(),
        )
        .unwrap();

        assert_eq!(resolved, installed);
    }

    /// A full miss reports every searched source so users can diagnose installs.
    #[test]
    fn reports_searched_locations_when_not_found() {
        let home = tempfile::tempdir().unwrap();
        let empty_path_dir = tempfile::tempdir().unwrap();

        let error = resolve_agent_cli_path(
            AgentCli::OpenCode,
            Some(&path_value(&[empty_path_dir.path()])),
            home.path(),
        )
        .unwrap_err();

        assert_eq!(error.classification(), ErrorClassification::NotFound);
        assert_eq!(
            error.public_error(),
            &PublicError::AgentCliNotFound(EmptyErrorParams {})
        );
        let fallback = home.path().join(".opencode").join("bin");
        assert_eq!(
            error.to_string(),
            format!(
                "opencode executable not found; searched PATH (1 directories) and {fallback}",
                fallback = fallback.display()
            )
        );
    }

    /// An absent PATH variable still resolves through the fixed install directory.
    #[test]
    fn resolves_without_path_variable() {
        let home = tempfile::tempdir().unwrap();
        let installed = write_executable(&home.path().join(".opencode").join("bin"), "opencode");

        let resolved =
            resolve_agent_cli_path(AgentCli::OpenCode, /*path_variable*/ None, home.path())
                .unwrap();

        assert_eq!(resolved, installed);
    }
}
