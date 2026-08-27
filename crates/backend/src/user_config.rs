use std::path::{Path, PathBuf};
use std::sync::Arc;

use ora_application::{ApplicationError, DeveloperMode, NetworkProxySettings, UserConfigService};
use ora_db::{RepositoryPool, SqliteUserConfigRepository};
use ora_logging::LogLevel;
use ora_runtime_settings::PreferredLogLevelStore;
use ora_user_config::{ConfigKey, UserConfigStore};

use crate::BackendError;
use crate::bootstrap::spawn_repository_work;

/// Owns Backend's concrete typed user-configuration composition.
#[derive(Clone)]
pub(crate) struct UserConfigApi {
    service: UserConfigService<SqliteUserConfigRepository>,
    store: UserConfigStore<SqliteUserConfigRepository>,
}

/// Gives runtime logging only the Backend-owned preferred-level capability it requires.
#[derive(Clone)]
pub struct BackendPreferredLogLevelStore {
    user_config: Arc<UserConfigApi>,
}

impl BackendPreferredLogLevelStore {
    pub(crate) fn new(user_config: Arc<UserConfigApi>) -> Self {
        Self { user_config }
    }
}

impl PreferredLogLevelStore for BackendPreferredLogLevelStore {
    type Error = BackendError;

    async fn load_preferred_level(&self) -> Result<LogLevel, Self::Error> {
        self.user_config.preferred_log_level().await
    }

    async fn save_preferred_level(&self, level: LogLevel) -> Result<(), Self::Error> {
        self.user_config.set_preferred_log_level(level).await?;
        Ok(())
    }
}

impl UserConfigApi {
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

    pub(crate) async fn developer_mode(&self) -> Result<DeveloperMode, BackendError> {
        let service = self.service.clone();
        spawn_repository_work(move || service.developer_mode().map_err(BackendError::from)).await
    }

    pub(crate) async fn set_developer_mode(
        &self,
        mode: DeveloperMode,
    ) -> Result<DeveloperMode, BackendError> {
        let service = self.service.clone();
        spawn_repository_work(move || service.set_developer_mode(mode).map_err(BackendError::from))
            .await
    }

    pub(crate) async fn preferred_log_level(&self) -> Result<LogLevel, BackendError> {
        let service = self.service.clone();
        spawn_repository_work(move || service.preferred_log_level().map_err(BackendError::from))
            .await
    }

    pub(crate) async fn set_preferred_log_level(
        &self,
        level: LogLevel,
    ) -> Result<LogLevel, BackendError> {
        let service = self.service.clone();
        spawn_repository_work(move || {
            service
                .set_preferred_log_level(level)
                .map_err(BackendError::from)
        })
        .await
    }
    /// Loads the optional configured network proxy settings.
    pub(crate) fn network_proxy_settings(
        &self,
    ) -> Result<Option<NetworkProxySettings>, BackendError> {
        self.service
            .network_proxy_settings()
            .map_err(BackendError::from)
    }

    /// Persists and returns the network proxy settings.
    pub(crate) fn set_network_proxy_settings(
        &self,
        settings: NetworkProxySettings,
    ) -> Result<NetworkProxySettings, BackendError> {
        self.service
            .set_network_proxy_settings(settings)
            .map_err(BackendError::from)
    }
}

fn user_config_repository_error(error: ora_application::RepositoryError) -> BackendError {
    BackendError::from(ApplicationError::UserConfigRepository { source: error })
}
