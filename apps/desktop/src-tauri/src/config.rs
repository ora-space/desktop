use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tempfile::NamedTempFile;
use thiserror::Error;

const CONFIG_VERSION: u32 = 1;
const CONFIG_FILE_NAME: &str = "config.json";
const DEFAULT_WORKTREE_DIRECTORY_NAME: &str = "worktrees";
const DEFAULT_DASHBOARD_HOST: &str = "127.0.0.1";
const DEFAULT_DASHBOARD_PORT: u16 = 8601;

/// Describes persisted non-sensitive Desktop runtime configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopConfig {
    version: u32,
    worktree_root: PathBuf,
    // The dashboard host/port are additive optional config carried with serde
    // defaults so a pre-existing config.json written before these fields existed
    // still deserializes without a version bump.
    #[serde(default = "default_dashboard_host")]
    dashboard_host: String,
    #[serde(default = "default_dashboard_port")]
    dashboard_port: u16,
}

fn default_dashboard_host() -> String {
    DEFAULT_DASHBOARD_HOST.to_string()
}

fn default_dashboard_port() -> u16 {
    DEFAULT_DASHBOARD_PORT
}

impl DesktopConfig {
    /// Returns the configured root used only when creating new task worktrees.
    pub fn worktree_root(&self) -> &Path {
        &self.worktree_root
    }

    /// Returns the host the embedded trace dashboard is served on (loopback by default).
    pub fn dashboard_host(&self) -> &str {
        &self.dashboard_host
    }

    /// Returns the port the embedded trace dashboard is served on.
    pub fn dashboard_port(&self) -> u16 {
        self.dashboard_port
    }
}

/// Owns the persisted Desktop configuration and its coherent in-memory snapshot.
#[derive(Clone)]
pub struct DesktopConfigStore {
    config_path: PathBuf,
    config: Arc<RwLock<DesktopConfig>>,
}

impl DesktopConfigStore {
    /// Loads an existing configuration or atomically creates the first-run default.
    pub fn load_or_create(app_data_directory: &Path) -> Result<Self, DesktopConfigError> {
        fs::create_dir_all(app_data_directory).map_err(|source| {
            DesktopConfigError::DirectoryCreate {
                path: app_data_directory.to_path_buf(),
                source,
            }
        })?;
        let config_path = app_data_directory.join(CONFIG_FILE_NAME);
        let config = if config_path.exists() {
            read_config(&config_path)?
        } else {
            let worktree_root = app_data_directory.join(DEFAULT_WORKTREE_DIRECTORY_NAME);
            fs::create_dir_all(&worktree_root).map_err(|source| {
                DesktopConfigError::DirectoryCreate {
                    path: worktree_root.clone(),
                    source,
                }
            })?;
            let config = DesktopConfig {
                version: CONFIG_VERSION,
                worktree_root,
                dashboard_host: default_dashboard_host(),
                dashboard_port: default_dashboard_port(),
            };
            persist_config(&config_path, &config)?;
            config
        };

        validate_config(&config)?;

        Ok(Self {
            config_path,
            config: Arc::new(RwLock::new(config)),
        })
    }

    /// Returns the current configuration snapshot without exposing the synchronization guard.
    pub fn snapshot(&self) -> Result<DesktopConfig, DesktopConfigError> {
        self.config
            .read()
            .map(|config| config.clone())
            .map_err(|_| DesktopConfigError::StateUnavailable)
    }

    /// Validates, persists, and publishes a new worktree creation root.
    pub fn set_worktree_root(&self, worktree_root: PathBuf) -> Result<(), DesktopConfigError> {
        validate_worktree_root(&worktree_root)?;
        let mut config = self
            .config
            .write()
            .map_err(|_| DesktopConfigError::StateUnavailable)?;
        let updated = DesktopConfig {
            version: CONFIG_VERSION,
            worktree_root,
            dashboard_host: config.dashboard_host.clone(),
            dashboard_port: config.dashboard_port,
        };

        persist_config(&self.config_path, &updated)?;
        *config = updated;
        Ok(())
    }

    /// Validates, persists, and publishes a new embedded dashboard endpoint.
    #[allow(dead_code, reason = "settings UI wiring is a later slice")]
    pub fn set_dashboard_endpoint(
        &self,
        host: String,
        port: u16,
    ) -> Result<(), DesktopConfigError> {
        validate_dashboard_endpoint(&host, port)?;
        let mut config = self
            .config
            .write()
            .map_err(|_| DesktopConfigError::StateUnavailable)?;
        let updated = DesktopConfig {
            version: CONFIG_VERSION,
            worktree_root: config.worktree_root.clone(),
            dashboard_host: host,
            dashboard_port: port,
        };

        persist_config(&self.config_path, &updated)?;
        *config = updated;
        Ok(())
    }
}

/// Reports explicit Desktop configuration read, validation, and persistence failures.
#[derive(Debug, Error)]
pub enum DesktopConfigError {
    #[error("failed to create Desktop configuration directory {path:?}")]
    DirectoryCreate {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read Desktop configuration {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to decode Desktop configuration {path:?}")]
    Decode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported Desktop configuration version {version}")]
    UnsupportedVersion { version: u32 },
    #[error("worktree root must be absolute: {path:?}")]
    WorktreeRootNotAbsolute { path: PathBuf },
    #[error("worktree root must be an existing directory: {path:?}")]
    WorktreeRootNotDirectory { path: PathBuf },
    #[error("dashboard host must be a non-empty loopback address")]
    DashboardHostEmpty,
    #[error("dashboard port must be non-zero")]
    DashboardPortZero,
    #[error("failed to persist Desktop configuration {path:?}")]
    Persist {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Desktop configuration state is unavailable")]
    StateUnavailable,
}

/// Reads and decodes one configuration without silently repairing malformed content.
fn read_config(config_path: &Path) -> Result<DesktopConfig, DesktopConfigError> {
    let file = fs::File::open(config_path).map_err(|source| DesktopConfigError::Read {
        path: config_path.to_path_buf(),
        source,
    })?;
    serde_json::from_reader(BufReader::new(file)).map_err(|source| DesktopConfigError::Decode {
        path: config_path.to_path_buf(),
        source,
    })
}

/// Validates the versioned configuration and its current worktree root.
fn validate_config(config: &DesktopConfig) -> Result<(), DesktopConfigError> {
    if config.version != CONFIG_VERSION {
        return Err(DesktopConfigError::UnsupportedVersion {
            version: config.version,
        });
    }

    validate_worktree_root(&config.worktree_root)?;
    validate_dashboard_endpoint(&config.dashboard_host, config.dashboard_port)
}

/// Rejects ambiguous or unavailable roots before they become active runtime configuration.
pub(crate) fn validate_worktree_root(worktree_root: &Path) -> Result<(), DesktopConfigError> {
    if !worktree_root.is_absolute() {
        return Err(DesktopConfigError::WorktreeRootNotAbsolute {
            path: worktree_root.to_path_buf(),
        });
    }
    if !worktree_root.is_dir() {
        return Err(DesktopConfigError::WorktreeRootNotDirectory {
            path: worktree_root.to_path_buf(),
        });
    }

    Ok(())
}

/// Rejects ambiguous dashboard endpoints before they become active runtime configuration.
pub(crate) fn validate_dashboard_endpoint(host: &str, port: u16) -> Result<(), DesktopConfigError> {
    if host.trim().is_empty() {
        return Err(DesktopConfigError::DashboardHostEmpty);
    }
    if port == 0 {
        return Err(DesktopConfigError::DashboardPortZero);
    }

    Ok(())
}
/// Writes a complete configuration to a sibling temporary file before atomic replacement.
fn persist_config(config_path: &Path, config: &DesktopConfig) -> Result<(), DesktopConfigError> {
    let directory = config_path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary =
        NamedTempFile::new_in(directory).map_err(|source| DesktopConfigError::Persist {
            path: config_path.to_path_buf(),
            source,
        })?;
    {
        let mut writer = BufWriter::new(temporary.as_file_mut());
        serde_json::to_writer_pretty(&mut writer, config).map_err(|source| {
            DesktopConfigError::Persist {
                path: config_path.to_path_buf(),
                source: std::io::Error::other(source),
            }
        })?;
        writer
            .write_all(b"\n")
            .map_err(|source| DesktopConfigError::Persist {
                path: config_path.to_path_buf(),
                source,
            })?;
        writer
            .flush()
            .map_err(|source| DesktopConfigError::Persist {
                path: config_path.to_path_buf(),
                source,
            })?;
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| DesktopConfigError::Persist {
            path: config_path.to_path_buf(),
            source,
        })?;
    temporary
        .persist(config_path)
        .map(|_| ())
        .map_err(|error| DesktopConfigError::Persist {
            path: config_path.to_path_buf(),
            source: error.error,
        })
}

#[cfg(test)]
mod tests {
    use super::{DesktopConfigError, DesktopConfigStore};
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Verifies first launch creates the versioned configuration and default worktree directory.
    #[test]
    fn creates_first_run_configuration() {
        let temporary = TempDir::new().expect("create temporary app data directory");
        let app_data = temporary.path().join("app-data");

        let store = DesktopConfigStore::load_or_create(&app_data)
            .expect("create first-run Desktop configuration");
        let snapshot = store.snapshot().expect("read Desktop configuration");
        let persisted = fs::read_to_string(app_data.join("config.json"))
            .expect("read persisted Desktop configuration");

        assert_eq!(snapshot.worktree_root(), app_data.join("worktrees"));
        assert!(snapshot.worktree_root().is_dir());
        assert_eq!(snapshot.dashboard_host(), "127.0.0.1");
        assert_eq!(snapshot.dashboard_port(), 8601);
        assert!(persisted.contains("\"version\": 1"));
        assert!(persisted.contains("\"worktreeRoot\""));
        assert!(persisted.contains("\"dashboardHost\""));
        assert!(persisted.contains("\"dashboardPort\""));
    }

    /// Verifies a valid user-selected directory is persisted and restored on the next launch.
    #[test]
    fn persists_selected_worktree_root() {
        let temporary = TempDir::new().expect("create temporary app data directory");
        let app_data = temporary.path().join("app-data");
        let selected = temporary.path().join("selected-worktrees");
        fs::create_dir_all(&selected).expect("create selected worktree root");
        let store = DesktopConfigStore::load_or_create(&app_data)
            .expect("create first-run Desktop configuration");

        store
            .set_worktree_root(selected.clone())
            .expect("persist selected worktree root");
        let reloaded =
            DesktopConfigStore::load_or_create(&app_data).expect("reload Desktop configuration");

        assert_eq!(
            reloaded
                .snapshot()
                .expect("read reloaded Desktop configuration")
                .worktree_root(),
            selected
        );
    }

    /// Verifies relative and missing roots are rejected without replacing the active snapshot.
    #[test]
    fn rejects_invalid_worktree_roots() {
        let temporary = TempDir::new().expect("create temporary app data directory");
        let store = DesktopConfigStore::load_or_create(temporary.path())
            .expect("create first-run Desktop configuration");
        let original = store.snapshot().expect("read original configuration");

        assert!(matches!(
            store.set_worktree_root(PathBuf::from("relative")),
            Err(DesktopConfigError::WorktreeRootNotAbsolute { .. })
        ));
        assert!(matches!(
            store.set_worktree_root(temporary.path().join("missing")),
            Err(DesktopConfigError::WorktreeRootNotDirectory { .. })
        ));
        assert_eq!(
            store.snapshot().expect("read unchanged configuration"),
            original
        );
    }

    /// Verifies malformed persisted configuration is fatal instead of silently reset.
    #[test]
    fn rejects_corrupt_configuration() {
        let temporary = TempDir::new().expect("create temporary app data directory");
        fs::write(temporary.path().join("config.json"), "{not json")
            .expect("write corrupt configuration");

        assert!(matches!(
            DesktopConfigStore::load_or_create(temporary.path()),
            Err(DesktopConfigError::Decode { .. })
        ));
    }

    /// Verifies a pre-existing config.json written before the dashboard endpoint fields existed
    /// still loads by applying the serde defaults for the missing fields.
    #[test]
    fn loads_legacy_configuration_without_dashboard_endpoint() {
        let temporary = TempDir::new().expect("create temporary app data directory");
        let worktrees = temporary.path().join("worktrees");
        fs::create_dir_all(&worktrees).expect("create legacy worktree root");
        let legacy = format!(
            "{{\"version\":1,\"worktreeRoot\":{}}}",
            serde_json::to_string(worktrees.to_string_lossy().as_ref()).unwrap()
        );
        fs::write(temporary.path().join("config.json"), legacy)
            .expect("write legacy configuration");

        let snapshot = DesktopConfigStore::load_or_create(temporary.path())
            .expect("load legacy Desktop configuration")
            .snapshot()
            .expect("read Desktop configuration");

        assert_eq!(snapshot.dashboard_host(), "127.0.0.1");
        assert_eq!(snapshot.dashboard_port(), 8601);
    }

    /// Verifies a user-selected dashboard endpoint is persisted and restored on the next launch.
    #[test]
    fn persists_selected_dashboard_endpoint() {
        let temporary = TempDir::new().expect("create temporary app data directory");
        let store = DesktopConfigStore::load_or_create(temporary.path())
            .expect("create first-run Desktop configuration");

        store
            .set_dashboard_endpoint("127.0.0.1".to_string(), 8602)
            .expect("persist selected dashboard endpoint");
        let snapshot =
            DesktopConfigStore::load_or_create(temporary.path()).expect("reload configuration");
        let snapshot = snapshot
            .snapshot()
            .expect("read reloaded Desktop configuration");

        assert_eq!(snapshot.dashboard_host(), "127.0.0.1");
        assert_eq!(snapshot.dashboard_port(), 8602);
    }

    /// Verifies empty hosts and zero ports are rejected without replacing the active snapshot.
    #[test]
    fn rejects_invalid_dashboard_endpoints() {
        let temporary = TempDir::new().expect("create temporary app data directory");
        let store = DesktopConfigStore::load_or_create(temporary.path())
            .expect("create first-run Desktop configuration");
        let original = store.snapshot().expect("read original configuration");

        assert!(matches!(
            store.set_dashboard_endpoint("  ".to_string(), 8601),
            Err(DesktopConfigError::DashboardHostEmpty)
        ));
        assert!(matches!(
            store.set_dashboard_endpoint("127.0.0.1".to_string(), 0),
            Err(DesktopConfigError::DashboardPortZero)
        ));
        assert_eq!(
            store.snapshot().expect("read unchanged configuration"),
            original
        );
    }
}
