use ora_logging::LogLevel;
use serde::{Deserialize, Serialize};

use crate::{ApplicationError, RepositoryError};

/// Stores the optional host-level network proxy used by configured marketplace sources.
///
/// Username and password are optional and are persisted only when the user supplies them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkProxySettings {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

/// Represents whether developer-facing settings are discoverable in the shared UI.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DeveloperMode {
    Enabled,
    #[default]
    Disabled,
}

impl DeveloperMode {
    /// Reports the boolean value used by transport contracts without weakening internal call sites.
    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// Supplies typed persistence for the supported shared user preferences.
///
/// Implementations are expected to own raw key/value encoding, return the documented defaults
/// when rows are absent, and reject malformed stored values rather than silently repairing them.
pub trait UserConfigRepository: Clone + Send + Sync + 'static {
    /// Loads the developer-mode preference, defaulting to disabled when no row exists.
    fn load_developer_mode(&self) -> Result<DeveloperMode, RepositoryError>;

    /// Atomically inserts or replaces the developer-mode preference.
    fn save_developer_mode(&self, mode: DeveloperMode) -> Result<(), RepositoryError>;

    /// Loads the preferred runtime log level, defaulting to info when no row exists.
    fn load_preferred_log_level(&self) -> Result<LogLevel, RepositoryError>;

    /// Atomically inserts or replaces the preferred runtime log level.
    fn save_preferred_log_level(&self, level: LogLevel) -> Result<(), RepositoryError>;

    /// Loads the optional network proxy settings, absent when the user has not configured one.
    fn load_network_proxy_settings(&self) -> Result<Option<NetworkProxySettings>, RepositoryError>;

    /// Atomically inserts or replaces the network proxy settings.
    fn save_network_proxy_settings(
        &self,
        settings: &NetworkProxySettings,
    ) -> Result<(), RepositoryError>;
}

/// Coordinates transport-independent reads and writes for shared user preferences.
#[derive(Clone)]
pub struct UserConfigService<R> {
    repository: R,
}

impl<R> UserConfigService<R>
where
    R: UserConfigRepository,
{
    /// Builds the service around an injected typed persistence port.
    pub fn new(repository: R) -> Self {
        Self { repository }
    }

    /// Returns the authoritative persisted developer-mode preference.
    pub fn developer_mode(&self) -> Result<DeveloperMode, ApplicationError> {
        self.repository
            .load_developer_mode()
            .map_err(ApplicationError::from_user_config_repository_error)
    }

    /// Persists and returns the authoritative developer-mode preference.
    pub fn set_developer_mode(
        &self,
        mode: DeveloperMode,
    ) -> Result<DeveloperMode, ApplicationError> {
        self.repository
            .save_developer_mode(mode)
            .map_err(ApplicationError::from_user_config_repository_error)?;
        Ok(mode)
    }

    /// Returns the authoritative preferred runtime log level.
    pub fn preferred_log_level(&self) -> Result<LogLevel, ApplicationError> {
        self.repository
            .load_preferred_log_level()
            .map_err(ApplicationError::from_user_config_repository_error)
    }

    /// Persists and returns the authoritative preferred runtime log level.
    pub fn set_preferred_log_level(&self, level: LogLevel) -> Result<LogLevel, ApplicationError> {
        self.repository
            .save_preferred_log_level(level)
            .map_err(ApplicationError::from_user_config_repository_error)?;
        Ok(level)
    }

    /// Returns the optional configured network proxy settings.
    pub fn network_proxy_settings(&self) -> Result<Option<NetworkProxySettings>, ApplicationError> {
        self.repository
            .load_network_proxy_settings()
            .map_err(ApplicationError::from_user_config_repository_error)
    }

    /// Persists and returns the authoritative network proxy settings.
    pub fn set_network_proxy_settings(
        &self,
        settings: NetworkProxySettings,
    ) -> Result<NetworkProxySettings, ApplicationError> {
        self.repository
            .save_network_proxy_settings(&settings)
            .map_err(ApplicationError::from_user_config_repository_error)?;
        Ok(settings)
    }
}
