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

/// Selects the host's route independently of its saved manual endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkProxyMode {
    System,
    Manual,
    Direct,
}

impl fmt::Display for NetworkProxyMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::System => "system",
            Self::Manual => "manual",
            Self::Direct => "direct",
        })
    }
}

impl FromStr for NetworkProxyMode {
    type Err = ParseNetworkProxyModeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "system" => Ok(Self::System),
            "manual" => Ok(Self::Manual),
            "direct" => Ok(Self::Direct),
            _ => Err(ParseNetworkProxyModeError),
        }
    }
}

#[derive(Clone, Copy, Debug, Error)]
#[error("unknown network proxy mode")]
pub struct ParseNetworkProxyModeError;

/// Contains the effective mode and the retained manual endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkProxyPreferences {
    pub mode: NetworkProxyMode,
    pub manual: Option<NetworkProxySettings>,
}

impl NetworkProxySettings {
    /// Checks the required fields before the network layer validates the endpoint.
    fn has_endpoint(&self) -> bool {
        self.port != 0 && !self.host.trim().is_empty()
    }
}

#[derive(Debug, Error)]
#[error("manual network proxy requires a host and port")]
struct InvalidManualProxy;

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
    /// Resolves old preferences without materializing default rows.
    pub fn network_proxy_preferences(&self) -> Result<NetworkProxyPreferences, ApplicationError> {
        let [mode, manual] = self
            .store
            .get_pair(ConfigKey::NetworkProxyMode, ConfigKey::NetworkProxySettings)
            .map_err(ApplicationError::from_user_config_repository_error)?;
        let mode = mode
            .map(|value| value.parse::<NetworkProxyMode>())
            .transpose()
            .map_err(|error| corrupt_value(ConfigKey::NetworkProxyMode, error))?;
        if let Some(mode @ (NetworkProxyMode::System | NetworkProxyMode::Direct)) = mode {
            return Ok(NetworkProxyPreferences { mode, manual: None });
        }
        let manual = manual
            .map(|value| value.parse_json::<NetworkProxySettings>())
            .transpose()
            .map_err(|error| corrupt_value(ConfigKey::NetworkProxySettings, error))?;
        let mode = match (mode, manual.as_ref()) {
            // Older builds only know the endpoint key and may clear it during a downgrade.
            (None | Some(NetworkProxyMode::Manual), None) => NetworkProxyMode::System,
            (None, Some(_)) => NetworkProxyMode::Manual,
            (Some(mode), _) => mode,
        };
        if mode == NetworkProxyMode::Manual
            && manual.as_ref().is_some_and(|manual| !manual.has_endpoint())
        {
            return Err(corrupt_value(
                ConfigKey::NetworkProxySettings,
                InvalidManualProxy,
            ));
        }
        Ok(NetworkProxyPreferences { mode, manual })
    }

    /// Commits a mode and its manual endpoint together, retaining the endpoint in other modes.
    pub fn set_network_proxy_preferences(
        &self,
        mode: NetworkProxyMode,
        manual: Option<NetworkProxySettings>,
    ) -> Result<(), ApplicationError> {
        if mode != NetworkProxyMode::Manual {
            return self
                .store
                .set_display(ConfigKey::NetworkProxyMode, mode)
                .map_err(ApplicationError::from_user_config_repository_error);
        }
        let manual = manual
            .filter(NetworkProxySettings::has_endpoint)
            .ok_or_else(|| corrupt_value(ConfigKey::NetworkProxySettings, InvalidManualProxy))?;
        self.store
            .set_with_json(
                ConfigKey::NetworkProxyMode,
                mode,
                ConfigKey::NetworkProxySettings,
                &manual,
            )
            .map_err(|error| {
                ApplicationError::from_user_config_repository_error(RepositoryError::new(error))
            })
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

    /// Forgets the explicitly saved configuration and returns to the default mode.
    pub fn clear_network_proxy_settings(&self) -> Result<(), ApplicationError> {
        self.store
            .delete_pair(ConfigKey::NetworkProxyMode, ConfigKey::NetworkProxySettings)
            .map_err(ApplicationError::from_user_config_repository_error)?;
        Ok(())
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
