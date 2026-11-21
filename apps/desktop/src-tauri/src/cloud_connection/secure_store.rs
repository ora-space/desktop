//! The operating system's secure store as the only place a token is kept at rest.

use ora_contracts::CloudAccount;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::secret::SessionToken;
use super::server::CloudServer;

/// Service name of every Cloud token entry; the entry's account is the server origin, so a token
/// can only be read back for the server that issued it.
const KEYRING_SERVICE: &str = "space.ora.desktop.cloud";

/// What is kept for one server: the token and the account it belongs to, so the account can be
/// shown before the server answers again (or while it is unreachable).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredSession {
    pub(crate) token: SessionToken,
    pub(crate) account: CloudAccount,
}

/// The secure store could not be used. The message is for display and logs and never holds the
/// token.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{0}")]
pub(crate) struct StoreError(pub(crate) String);

/// Keeps at most one session per server origin. Calls may block (Secret Service talks over D-Bus
/// and may wait for an unlock prompt), so callers run them off the async workers.
pub(crate) trait SecureStore: Send + Sync + 'static {
    /// Reads the session stored for `server`, if any.
    fn load(&self, server: &CloudServer) -> Result<Option<StoredSession>, StoreError>;
    /// Replaces the session stored for `server`.
    fn save(&self, server: &CloudServer, session: &StoredSession) -> Result<(), StoreError>;
    /// Removes the session stored for `server`; removing nothing is not an error.
    fn delete(&self, server: &CloudServer) -> Result<(), StoreError>;
}

/// Windows Credential Manager or the Linux Secret Service, through `keyring`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct KeyringStore;

/// The serialized entry. Field names are part of what earlier versions wrote.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntryValue {
    token: String,
    account_id: String,
    display_name: String,
}

impl KeyringStore {
    /// Opens the entry for one server.
    fn entry(server: &CloudServer) -> Result<keyring::Entry, StoreError> {
        keyring::Entry::new(KEYRING_SERVICE, server.origin()).map_err(store_error)
    }
}

/// Keeps the platform's explanation without anything that could be the secret.
fn store_error(error: keyring::Error) -> StoreError {
    StoreError(error.to_string())
}

impl SecureStore for KeyringStore {
    /// An unreadable entry is reported rather than treated as signed out, so a corrupted or
    /// foreign value is never silently overwritten by the next sign-in's assumptions.
    fn load(&self, server: &CloudServer) -> Result<Option<StoredSession>, StoreError> {
        let raw = match Self::entry(server)?.get_password() {
            Ok(raw) => raw,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(error) => return Err(store_error(error)),
        };
        let value: EntryValue = serde_json::from_str(&raw)
            .map_err(|_| StoreError("the stored Cloud credential is unreadable".to_owned()))?;
        Ok(Some(StoredSession {
            token: SessionToken::new(value.token),
            account: CloudAccount {
                id: value.account_id,
                display_name: value.display_name,
            },
        }))
    }

    /// Writes the whole entry at once.
    fn save(&self, server: &CloudServer, session: &StoredSession) -> Result<(), StoreError> {
        let value = serde_json::to_string(&EntryValue {
            token: session.token.expose().to_owned(),
            account_id: session.account.id.clone(),
            display_name: session.account.display_name.clone(),
        })
        .map_err(|error| StoreError(error.to_string()))?;
        Self::entry(server)?
            .set_password(&value)
            .map_err(store_error)
    }

    /// Deletes the entry if present.
    fn delete(&self, server: &CloudServer) -> Result<(), StoreError> {
        match Self::entry(server)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(store_error(error)),
        }
    }
}
