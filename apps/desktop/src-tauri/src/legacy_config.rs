use std::fs;
use std::path::{Path, PathBuf};

use ora_backend::{Backend, BackendError};
use serde::Deserialize;
use thiserror::Error;

const CONFIG_VERSION: u32 = 1;
const CONFIG_FILE_NAME: &str = "config.json";
const ORA_DIRECTORY_NAME: &str = ".ora";
const DEFAULT_WORKTREE_DIRECTORY_NAME: &str = "worktrees";

/// Reads only the field needed to move users off the retired Desktop JSON store.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyDesktopConfig {
    version: u32,
    worktree_root: PathBuf,
}

pub(crate) fn default_worktree_root(home_directory: &Path) -> PathBuf {
    home_directory
        .join(ORA_DIRECTORY_NAME)
        .join(DEFAULT_WORKTREE_DIRECTORY_NAME)
}

/// Initializes the SQLite row once, then removes the retired JSON file.
///
/// A pre-existing SQLite row is authoritative and bypasses JSON decoding, which lets a later
/// cleanup retry remove a corrupt stale file without risking the active value.
pub(crate) fn migrate(
    backend: &Backend,
    app_data_directory: &Path,
    home_directory: &Path,
) -> Result<(), LegacyConfigError> {
    fs::create_dir_all(app_data_directory).map_err(|source| {
        LegacyConfigError::DirectoryCreate {
            path: app_data_directory.to_path_buf(),
            source,
        }
    })?;
    let config_path = app_data_directory.join(CONFIG_FILE_NAME);

    if backend.persisted_worktree_root()?.is_some() {
        remove_legacy_file(&config_path)?;
        return Ok(());
    }

    let worktree_root = if config_path.exists() {
        read_legacy_worktree_root(&config_path)?
    } else {
        let default = default_worktree_root(home_directory);
        fs::create_dir_all(&default).map_err(|source| LegacyConfigError::DirectoryCreate {
            path: default.clone(),
            source,
        })?;
        default
    };

    validate_worktree_root(&worktree_root)?;
    backend.set_worktree_root(worktree_root)?;
    remove_legacy_file(&config_path)
}

fn read_legacy_worktree_root(path: &Path) -> Result<PathBuf, LegacyConfigError> {
    let bytes = fs::read(path).map_err(|source| LegacyConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let config: LegacyDesktopConfig =
        serde_json::from_slice(&bytes).map_err(|source| LegacyConfigError::Decode {
            path: path.to_path_buf(),
            source,
        })?;
    if config.version != CONFIG_VERSION {
        return Err(LegacyConfigError::UnsupportedVersion {
            version: config.version,
        });
    }
    Ok(config.worktree_root)
}

fn validate_worktree_root(path: &Path) -> Result<(), LegacyConfigError> {
    if !path.is_absolute() {
        return Err(LegacyConfigError::WorktreeRootNotAbsolute {
            path: path.to_path_buf(),
        });
    }
    if !path.is_dir() {
        return Err(LegacyConfigError::WorktreeRootNotDirectory {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn remove_legacy_file(path: &Path) -> Result<(), LegacyConfigError> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(path).map_err(|source| LegacyConfigError::Remove {
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug, Error)]
pub enum LegacyConfigError {
    #[error("failed to create Desktop configuration directory {path:?}")]
    DirectoryCreate {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read legacy Desktop configuration {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to decode legacy Desktop configuration {path:?}")]
    Decode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported legacy Desktop configuration version {version}")]
    UnsupportedVersion { version: u32 },
    #[error("worktree root must be absolute: {path:?}")]
    WorktreeRootNotAbsolute { path: PathBuf },
    #[error("worktree root must be an existing directory: {path:?}")]
    WorktreeRootNotDirectory { path: PathBuf },
    #[error("failed to remove migrated Desktop configuration {path:?}")]
    Remove {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read or persist worktree configuration")]
    Backend(#[from] BackendError),
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{LegacyConfigError, migrate, read_legacy_worktree_root};
    use ora_backend::{Backend, BackendPaths};
    use tempfile::TempDir;

    #[test]
    fn reads_worktree_root_from_legacy_shape() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path().join("worktrees");
        fs::create_dir_all(&root).unwrap();
        let path = temporary.path().join("config.json");
        fs::write(
            &path,
            format!(
                r#"{{"version":1,"worktreeRoot":{}}}"#,
                serde_json::to_string(&root).unwrap()
            ),
        )
        .unwrap();

        assert_eq!(read_legacy_worktree_root(&path).unwrap(), root);
    }

    #[test]
    fn rejects_corrupt_or_unknown_version_legacy_files() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("config.json");
        fs::write(&path, "{not json").unwrap();
        assert!(matches!(
            read_legacy_worktree_root(&path),
            Err(LegacyConfigError::Decode { .. })
        ));

        fs::write(&path, r#"{"version":2,"worktreeRoot":"C:/worktrees"}"#).unwrap();
        assert!(matches!(
            read_legacy_worktree_root(&path),
            Err(LegacyConfigError::UnsupportedVersion { version: 2 })
        ));
    }

    #[test]
    fn migrates_legacy_root_into_sqlite_and_removes_the_file() {
        let temporary = TempDir::new().unwrap();
        let app_data = temporary.path().join("app-data");
        let home = temporary.path().join("home");
        let selected = temporary.path().join("selected-worktrees");
        fs::create_dir_all(&selected).unwrap();
        fs::create_dir_all(&app_data).unwrap();
        write_legacy(&app_data.join("config.json"), &selected);

        let backend = Backend::open(backend_paths(&app_data, &home)).unwrap();
        migrate(&backend, &app_data, &home).unwrap();

        assert_eq!(backend.worktree_root().unwrap(), selected);
        assert_eq!(
            backend.persisted_worktree_root().unwrap(),
            Some(selected.clone())
        );
        assert!(!app_data.join("config.json").exists());

        drop(backend);
        let reopened = Backend::open(backend_paths(&app_data, &home)).unwrap();
        assert_eq!(reopened.worktree_root().unwrap(), selected);
    }

    #[test]
    fn persisted_sqlite_root_wins_over_a_corrupt_legacy_file() {
        let temporary = TempDir::new().unwrap();
        let app_data = temporary.path().join("app-data");
        let home = temporary.path().join("home");
        let selected = temporary.path().join("selected-worktrees");
        fs::create_dir_all(&selected).unwrap();

        let backend = Backend::open(backend_paths(&app_data, &home)).unwrap();
        backend.set_worktree_root(selected.clone()).unwrap();
        fs::write(app_data.join("config.json"), "{not json").unwrap();

        migrate(&backend, &app_data, &home).unwrap();

        assert_eq!(backend.worktree_root().unwrap(), selected);
        assert!(!app_data.join("config.json").exists());
    }

    #[test]
    fn corrupt_first_migration_keeps_the_legacy_file_and_database_empty() {
        let temporary = TempDir::new().unwrap();
        let app_data = temporary.path().join("app-data");
        let home = temporary.path().join("home");
        fs::create_dir_all(&app_data).unwrap();
        let config_path = app_data.join("config.json");
        fs::write(&config_path, "{not json").unwrap();
        let backend = Backend::open(backend_paths(&app_data, &home)).unwrap();

        assert!(matches!(
            migrate(&backend, &app_data, &home),
            Err(LegacyConfigError::Decode { .. })
        ));
        assert!(config_path.exists());
        assert_eq!(backend.persisted_worktree_root().unwrap(), None);
    }

    fn write_legacy(path: &Path, worktree_root: &Path) {
        fs::write(
            path,
            format!(
                r#"{{"version":1,"worktreeRoot":{}}}"#,
                serde_json::to_string(worktree_root).unwrap()
            ),
        )
        .unwrap();
    }

    fn backend_paths(app_data: &Path, home: &Path) -> BackendPaths {
        BackendPaths {
            database_path: app_data.join("ora.sqlite3"),
            data_directory: home.join(".ora"),
            deno_path: PathBuf::from("deno"),
            worktree_root: home.join(".ora").join("worktrees"),
            home_directory: home.to_path_buf(),
            relative_path_base: app_data.to_path_buf(),
            sessions_root: app_data.join("sessions"),
            skills_root: app_data.join("atoms").join("skills"),
            ripgrep_path: PathBuf::from("rg"),
            timezone: chrono_tz::UTC,
        }
    }
}
