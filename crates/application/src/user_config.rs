use std::fmt;
use std::str::FromStr;

use ora_logging::LogLevel;
use ora_user_config::{ConfigKey, UserConfigRepository, UserConfigStore};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
use crate::{ApplicationError, RepositoryError};

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

impl fmt::Display for DeveloperMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Enabled => "true",
            Self::Disabled => "false",
        })
    }
}

impl FromStr for DeveloperMode {
    type Err = ParseDeveloperModeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "true" => Ok(Self::Enabled),
            "false" => Ok(Self::Disabled),
            _ => Err(ParseDeveloperModeError),
        }
    }
}

#[derive(Clone, Copy, Debug, Error)]
#[error("developer mode must be stored as canonical true or false")]
pub struct ParseDeveloperModeError;

/// Coordinates business-owned preference types over the generic user-configuration module.
#[derive(Clone)]
pub struct UserConfigService<R> {
    store: UserConfigStore<R>,
}

impl<R> UserConfigService<R>
where
    R: UserConfigRepository<Error = RepositoryError>,
{
    pub fn new(repository: R) -> Self {
        Self {
            store: UserConfigStore::new(repository),
        }
    }

    /// Returns the authoritative persisted developer-mode preference.
    pub fn developer_mode(&self) -> Result<DeveloperMode, ApplicationError> {
        match self
            .store
            .get(ConfigKey::DeveloperMode)
            .map_err(ApplicationError::from_user_config_repository_error)?
        {
            None => Ok(DeveloperMode::Disabled),
            Some(value) => value
                .parse()
                .map_err(|error| corrupt_value(ConfigKey::DeveloperMode, error)),
        }
    }

    /// Persists and returns the authoritative developer-mode preference.
    pub fn set_developer_mode(
        &self,
        mode: DeveloperMode,
    ) -> Result<DeveloperMode, ApplicationError> {
        self.store
            .set_display(ConfigKey::DeveloperMode, mode)
            .map_err(ApplicationError::from_user_config_repository_error)?;
        Ok(mode)
    }

    /// Returns the authoritative preferred runtime log level.
    pub fn preferred_log_level(&self) -> Result<LogLevel, ApplicationError> {
        let Some(value) = self
            .store
            .get(ConfigKey::LogLevel)
            .map_err(ApplicationError::from_user_config_repository_error)?
        else {
            return Ok(LogLevel::Info);
        };
        let level = value
            .parse::<LogLevel>()
            .map_err(|error| corrupt_value(ConfigKey::LogLevel, error))?;
        if value.as_str() != level.as_str() {
            return Err(corrupt_value(ConfigKey::LogLevel, NonCanonicalConfigValue));
        }
        Ok(level)
    }

    /// Persists and returns the authoritative preferred runtime log level.
    pub fn set_preferred_log_level(&self, level: LogLevel) -> Result<LogLevel, ApplicationError> {
        self.store
            .set_display(ConfigKey::LogLevel, level)
            .map_err(ApplicationError::from_user_config_repository_error)?;
        Ok(level)
    }
    /// Returns the optional configured network proxy settings.
    pub fn network_proxy_settings(&self) -> Result<Option<NetworkProxySettings>, ApplicationError> {
        match self
            .store
            .get(ConfigKey::NetworkProxySettings)
            .map_err(ApplicationError::from_user_config_repository_error)?
        {
            None => Ok(None),
            Some(value) => value
                .parse_json()
                .map(Some)
                .map_err(|error| corrupt_value(ConfigKey::NetworkProxySettings, error)),
        }
    }

    /// Persists and returns the authoritative network proxy settings.
    pub fn set_network_proxy_settings(
        &self,
        settings: NetworkProxySettings,
    ) -> Result<NetworkProxySettings, ApplicationError> {
        self.store
            .set_json(ConfigKey::NetworkProxySettings, &settings)
            .map_err(|error| {
                ApplicationError::from_user_config_repository_error(RepositoryError::new(error))
            })?;
        Ok(settings)
    }
}

#[derive(Clone, Copy, Debug, Error)]
#[error("configuration value is not stored in canonical form")]
struct NonCanonicalConfigValue;

fn corrupt_value(
    key: ConfigKey,
    error: impl std::error::Error + Send + Sync + 'static,
) -> ApplicationError {
    ApplicationError::from_user_config_repository_error(RepositoryError::new(
        CorruptUserConfigValue {
            key: key.as_str(),
            source: Box::new(error),
        },
    ))
}

#[derive(Debug, Error)]
#[error("user configuration value for {key} is corrupt")]
struct CorruptUserConfigValue {
    key: &'static str,
    #[source]
    source: Box<dyn std::error::Error + Send + Sync + 'static>,
}
