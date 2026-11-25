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
    /// Returns the saved probe target; only an absent value uses the first-use default.
    pub async fn network_proxy_check_url(&self) -> Result<String, BackendError> {
        let store = self.store.clone();
        spawn_repository_work(move || {
            let saved = store
                .get(ConfigKey::NetworkProxyCheckUrl)
                .map_err(user_config_repository_error)?;
            match saved {
                Some(value) => validated_proxy_check_url(value.as_str()),
                None => Ok("https://google.com".to_owned()),
            }
        })
        .await
    }

    /// Saves only the explicitly confirmed probe URL, independently of proxy form edits.
    pub async fn set_network_proxy_check_url(&self, url: String) -> Result<String, BackendError> {
        let url = validated_proxy_check_url(&url)?;
        let store = self.store.clone();
        spawn_repository_work(move || {
            store
                .set_display(ConfigKey::NetworkProxyCheckUrl, &url)
                .map_err(user_config_repository_error)?;
            Ok(url)
        })
        .await
    }

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
    /// Loads the optional proxy without blocking an async worker on SQLite.
    pub async fn network_proxy_settings(
        &self,
    ) -> Result<Option<NetworkProxySettings>, BackendError> {
        let service = self.service.clone();
        spawn_repository_work(move || service.network_proxy_settings().map_err(BackendError::from))
            .await
    }

    /// Persists and returns the proxy without blocking an async worker on SQLite.
    pub async fn set_network_proxy_settings(
        &self,
        settings: NetworkProxySettings,
    ) -> Result<NetworkProxySettings, BackendError> {
        let service = self.service.clone();
        spawn_repository_work(move || {
            service
                .set_network_proxy_settings(settings)
                .map_err(BackendError::from)
        })
        .await
    }

    /// Removes the configured proxy without blocking an async worker on SQLite.
    pub async fn clear_network_proxy_settings(&self) -> Result<(), BackendError> {
        let service = self.service.clone();
        spawn_repository_work(move || {
            service
                .clear_network_proxy_settings()
                .map_err(BackendError::from)
        })
        .await
    }

    /// Probes `url` through the supplied proxy without persisting form edits.
    pub async fn check_network_proxy_settings(
        &self,
        settings: NetworkProxySettings,
        url: String,
    ) -> Result<ora_contracts::CheckProxySettingsResponse, BackendError> {
        crate::proxy::check_proxy(&settings, &url).await
    }

    /// Loads the Cloud server origin Desktop connects to, if one was saved.
    ///
    /// The value is the origin already normalized by the Desktop Cloud connection; it is not a
    /// secret, so it lives with the other preferences while the token stays in the OS store.
    pub async fn cloud_server(&self) -> Result<Option<String>, BackendError> {
        let store = self.store.clone();
        spawn_repository_work(move || {
            store
                .get(ConfigKey::CloudServer)
                .map(|value| value.map(|value| value.as_str().to_owned()))
                .map_err(user_config_repository_error)
        })
        .await
    }

    /// Persists the Cloud server origin chosen in Settings.
    pub async fn set_cloud_server(&self, origin: String) -> Result<(), BackendError> {
        let store = self.store.clone();
        spawn_repository_work(move || {
            store
                .set_display(ConfigKey::CloudServer, origin)
                .map_err(user_config_repository_error)
        })
        .await
    }

    /// Forgets the Cloud server origin, leaving Desktop unconnected.
    pub async fn clear_cloud_server(&self) -> Result<(), BackendError> {
        let store = self.store.clone();
        spawn_repository_work(move || {
            store
                .delete(ConfigKey::CloudServer)
                .map_err(user_config_repository_error)
        })
        .await
    }

    /// Restricts runtime logging to its preferred-level persistence capability.
    pub fn preferred_log_level_store(&self) -> BackendPreferredLogLevelStore {
        BackendPreferredLogLevelStore {
            settings: self.clone(),
        }
    }
}

/// Validates without echoing a possibly private target into error messages.
fn validated_proxy_check_url(value: &str) -> Result<String, BackendError> {
    let value = value.trim();
    let valid = url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
    if !valid {
        return Err(BackendError::invalid_proxy_settings(
            "probe URL must be an absolute HTTP(S) URL".to_owned(),
        ));
    }
    Ok(value.to_owned())
}

/// Keeps storage diagnostics internal while preserving the common public error projection.
fn user_config_repository_error(error: ora_application::RepositoryError) -> BackendError {
    BackendError::from(ApplicationError::UserConfigRepository { source: error })
}

#[cfg(test)]
mod tests;
