//! Typed settings use cases; persistence and worktree configuration remain internal.

use std::path::{Path, PathBuf};

use ora_application::{ApplicationError, DeveloperMode, NetworkProxySettings, UserConfigService};
use ora_db::{RepositoryPool, SqliteUserConfigRepository};
use ora_logging::LogLevel;
use ora_runtime_settings::PreferredLogLevelStore;
use ora_user_config::{ConfigKey, UserConfigStore};

use crate::BackendError;
use crate::repository_work::spawn_repository_work;

/// Provides persisted preferences without exposing repositories or application runtime ownership.
#[derive(Clone)]
pub struct Settings {
    service: UserConfigService<SqliteUserConfigRepository>,
    store: UserConfigStore<SqliteUserConfigRepository>,
}

/// Gives runtime logging only the Backend-owned preferred-level capability it requires.
#[derive(Clone)]
pub struct BackendPreferredLogLevelStore {
    settings: Settings,
}

impl PreferredLogLevelStore for BackendPreferredLogLevelStore {
    type Error = BackendError;

    /// Reads only the persistence capability required by runtime logging.
    async fn load_preferred_level(&self) -> Result<LogLevel, Self::Error> {
        self.settings.preferred_log_level().await
    }

    /// Keeps runtime filter changes independent of the complete Backend handle.
    async fn save_preferred_level(&self, level: LogLevel) -> Result<(), Self::Error> {
        self.settings.set_preferred_log_level(level).await?;
        Ok(())
    }
}

impl Settings {
    pub(crate) fn new(pool: RepositoryPool) -> Self {
        let repository = SqliteUserConfigRepository::new(pool);
        Self {
            service: UserConfigService::new(repository.clone()),
            store: UserConfigStore::new(repository),
        }
    }

    /// Reads the persisted worktree creation root without inventing a default.
    pub(crate) fn worktree_root(&self) -> Result<Option<PathBuf>, BackendError> {
        self.store
            .get(ConfigKey::WorktreeRoot)
            .map(|value| value.map(|value| PathBuf::from(value.as_str())))
            .map_err(user_config_repository_error)
    }

    /// Persists the canonical path selected by the worktree business module.
    pub(crate) fn set_worktree_root(&self, root: &Path) -> Result<(), BackendError> {
        self.store
            .set_display(ConfigKey::WorktreeRoot, root.display())
            .map_err(user_config_repository_error)
    }

    /// Loads the preference without blocking an async worker on SQLite.
    pub async fn developer_mode(&self) -> Result<DeveloperMode, BackendError> {
        let service = self.service.clone();
        spawn_repository_work(move || service.developer_mode().map_err(BackendError::from)).await
    }

    /// Persists and returns the authoritative developer-mode preference.
    pub async fn set_developer_mode(
        &self,
        mode: DeveloperMode,
    ) -> Result<DeveloperMode, BackendError> {
        let service = self.service.clone();
        spawn_repository_work(move || service.set_developer_mode(mode).map_err(BackendError::from))
            .await
    }

    /// Loads the preferred level; the process-wide effective filter belongs to runtime settings.
    pub async fn preferred_log_level(&self) -> Result<LogLevel, BackendError> {
        let service = self.service.clone();
        spawn_repository_work(move || service.preferred_log_level().map_err(BackendError::from))
            .await
    }

    /// Persists the preferred level without changing the process-wide logging filter itself.
    pub async fn set_preferred_log_level(&self, level: LogLevel) -> Result<LogLevel, BackendError> {
        let service = self.service.clone();
        spawn_repository_work(move || {
            service
                .set_preferred_log_level(level)
                .map_err(BackendError::from)
        })
        .await
    }
    /// Loads the optional configured network proxy settings.
    pub fn network_proxy_settings(&self) -> Result<Option<NetworkProxySettings>, BackendError> {
        self.service
            .network_proxy_settings()
            .map_err(BackendError::from)
    }

    /// Persists and returns the network proxy settings.
    pub fn set_network_proxy_settings(
        &self,
        settings: NetworkProxySettings,
    ) -> Result<NetworkProxySettings, BackendError> {
        self.service
            .set_network_proxy_settings(settings)
            .map_err(BackendError::from)
    }

    /// Removes the configured network proxy.
    pub fn clear_network_proxy_settings(&self) -> Result<(), BackendError> {
        self.service
            .clear_network_proxy_settings()
            .map_err(BackendError::from)
    }

    /// Probes `url` through the supplied proxy without persisting form edits.
    pub async fn check_network_proxy_settings(
        &self,
        settings: NetworkProxySettings,
        url: String,
    ) -> Result<ora_contracts::CheckProxySettingsResponse, BackendError> {
        crate::proxy::check_proxy(&settings, &url).await
    }

    /// Restricts runtime logging to its preferred-level persistence capability.
    pub fn preferred_log_level_store(&self) -> BackendPreferredLogLevelStore {
        BackendPreferredLogLevelStore {
            settings: self.clone(),
        }
    }
}

/// Keeps storage diagnostics internal while preserving the common public error projection.
fn user_config_repository_error(error: ora_application::RepositoryError) -> BackendError {
    BackendError::from(ApplicationError::UserConfigRepository { source: error })
}

#[cfg(test)]
mod tests;
