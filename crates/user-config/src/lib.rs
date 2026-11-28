use std::fmt::Display;
use std::str::FromStr;

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

/// Names every persisted user preference without spreading string literals across callers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigKey {
    /// Normalized origin of the Cloud server Desktop signs in to; the token is never stored here.
    CloudServer,
    DeveloperMode,
    LogLevel,
    NetworkProxySettings,
    NetworkProxyMode,
    NetworkProxyCheckUrl,
    WorktreeRoot,
}

impl ConfigKey {
    /// Returns the stable SQLite key used by every storage adapter.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CloudServer => "cloud_server",
            Self::DeveloperMode => "developer_mode",
            Self::NetworkProxySettings => "network_proxy_settings",
            Self::NetworkProxyMode => "network_proxy_mode",
            Self::NetworkProxyCheckUrl => "network_proxy_check_url",
            Self::LogLevel => "log_level",
            Self::WorktreeRoot => "worktree_root",
        }
    }
}

/// Owns one raw persisted value and centralizes conversions above the storage adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigValue(String);

impl ConfigValue {
    /// Wraps a raw value returned by a persistence adapter.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrows the raw value without allocating.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parses a scalar value through its business-owned FromStr implementation.
    pub fn parse<T>(&self) -> Result<T, T::Err>
    where
        T: FromStr,
    {
        self.0.parse()
    }

    /// Deserializes a structured value through its business-owned Serde schema.
    pub fn parse_json<T>(&self) -> Result<T, serde_json::Error>
    where
        T: DeserializeOwned,
    {
        serde_json::from_str(&self.0)
    }
}

/// Supplies raw key/value persistence without knowing any business configuration type.
pub trait UserConfigRepository: Clone + Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    fn get_value(&self, key: &str) -> Result<Option<String>, Self::Error>;

    /// Reads two related keys from one snapshot.
    fn get_pair(&self, first: &str, second: &str) -> Result<[Option<String>; 2], Self::Error>;

    /// Writes two related values atomically.
    fn set_pair(&self, first: (&str, &str), second: (&str, &str)) -> Result<(), Self::Error>;
    fn set_value(&self, key: &str, value: &str) -> Result<(), Self::Error>;
    fn delete_value(&self, key: &str) -> Result<(), Self::Error>;

    /// Deletes related preferences together.
    fn delete_pair(&self, first: &str, second: &str) -> Result<(), Self::Error>;
}

/// Provides typed keys and reusable value conversions over a raw KV adapter.
#[derive(Clone, Debug)]
pub struct UserConfigStore<R> {
    repository: R,
}

impl<R> UserConfigStore<R>
where
    R: UserConfigRepository,
{
    pub fn new(repository: R) -> Self {
        Self { repository }
    }

    pub fn get(&self, key: ConfigKey) -> Result<Option<ConfigValue>, R::Error> {
        self.repository
            .get_value(key.as_str())
            .map(|value| value.map(ConfigValue::new))
    }

    /// Reads two related typed keys from one snapshot.
    pub fn get_pair(
        &self,
        first: ConfigKey,
        second: ConfigKey,
    ) -> Result<[Option<ConfigValue>; 2], R::Error> {
        self.repository
            .get_pair(first.as_str(), second.as_str())
            .map(|values| values.map(|value| value.map(ConfigValue::new)))
    }

    /// Writes a selector and a JSON payload atomically.
    pub fn set_with_json<T: Serialize>(
        &self,
        key: ConfigKey,
        value: impl Display,
        payload_key: ConfigKey,
        payload: &T,
    ) -> Result<(), SetJsonError<R::Error>> {
        let payload = serde_json::to_string(payload).map_err(SetJsonError::Serialize)?;
        self.repository
            .set_pair(
                (key.as_str(), &value.to_string()),
                (payload_key.as_str(), &payload),
            )
            .map_err(SetJsonError::Repository)
    }

    pub fn set_display(&self, key: ConfigKey, value: impl Display) -> Result<(), R::Error> {
        self.repository.set_value(key.as_str(), &value.to_string())
    }

    pub fn set_json<T>(&self, key: ConfigKey, value: &T) -> Result<(), SetJsonError<R::Error>>
    where
        T: Serialize,
    {
        let serialized = serde_json::to_string(value).map_err(SetJsonError::Serialize)?;
        self.repository
            .set_value(key.as_str(), &serialized)
            .map_err(SetJsonError::Repository)
    }

    /// Removes two related typed keys in one operation.
    pub fn delete_pair(&self, first: ConfigKey, second: ConfigKey) -> Result<(), R::Error> {
        self.repository.delete_pair(first.as_str(), second.as_str())
    }

    pub fn delete(&self, key: ConfigKey) -> Result<(), R::Error> {
        self.repository.delete_value(key.as_str())
    }
}

/// Preserves whether a structured write failed before or during persistence.
#[derive(Debug, Error)]
pub enum SetJsonError<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    #[error("failed to serialize user configuration value")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to persist user configuration value")]
    Repository(#[source] E),
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    use pretty_assertions::assert_eq;
    use serde::{Deserialize, Serialize};

    use super::{ConfigKey, ConfigValue, UserConfigRepository, UserConfigStore};

    #[derive(Clone, Default)]
    struct MemoryRepository(Arc<Mutex<HashMap<String, String>>>);

    impl UserConfigRepository for MemoryRepository {
        type Error = Infallible;

        fn get_pair(&self, first: &str, second: &str) -> Result<[Option<String>; 2], Self::Error> {
            let values = self.0.lock().unwrap();
            Ok([values.get(first).cloned(), values.get(second).cloned()])
        }

        fn set_pair(&self, first: (&str, &str), second: (&str, &str)) -> Result<(), Self::Error> {
            let mut values = self.0.lock().unwrap();
            for (key, value) in [first, second] {
                values.insert(key.to_owned(), value.to_owned());
            }
            Ok(())
        }

        fn get_value(&self, key: &str) -> Result<Option<String>, Self::Error> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }

        fn set_value(&self, key: &str, value: &str) -> Result<(), Self::Error> {
            self.0
                .lock()
                .unwrap()
                .insert(key.to_owned(), value.to_owned());
            Ok(())
        }

        fn delete_pair(&self, first: &str, second: &str) -> Result<(), Self::Error> {
            let mut values = self.0.lock().unwrap();
            values.remove(first);
            values.remove(second);
            Ok(())
        }

        fn delete_value(&self, key: &str) -> Result<(), Self::Error> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct Example {
        enabled: bool,
    }

    #[test]
    fn maps_keys_to_stable_storage_names() {
        assert_eq!(
            ConfigKey::NetworkProxySettings.as_str(),
            "network_proxy_settings"
        );
        assert_eq!(ConfigKey::DeveloperMode.as_str(), "developer_mode");
        assert_eq!(ConfigKey::LogLevel.as_str(), "log_level");
        assert_eq!(ConfigKey::WorktreeRoot.as_str(), "worktree_root");
    }

    #[test]
    fn parses_scalar_and_json_values() {
        assert_eq!(ConfigValue::new("42").parse::<u16>(), Ok(42));
        assert_eq!(
            ConfigValue::new(r#"{"enabled":true}"#)
                .parse_json::<Example>()
                .unwrap(),
            Example { enabled: true }
        );
    }

    #[test]
    fn stores_display_and_json_values_and_deletes_them() {
        let store = UserConfigStore::new(MemoryRepository::default());
        store.set_display(ConfigKey::LogLevel, "debug").unwrap();
        assert_eq!(
            store.get(ConfigKey::LogLevel).unwrap().unwrap().as_str(),
            "debug"
        );

        store
            .set_json(ConfigKey::DeveloperMode, &Example { enabled: true })
            .unwrap();
        assert_eq!(
            store
                .get(ConfigKey::DeveloperMode)
                .unwrap()
                .unwrap()
                .parse_json::<Example>()
                .unwrap(),
            Example { enabled: true }
        );

        store.delete(ConfigKey::LogLevel).unwrap();
        assert_eq!(store.get(ConfigKey::LogLevel).unwrap(), None);
    }
}
