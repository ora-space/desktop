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

/// Windows Credential Manager or the Linux Secret Service.
///
/// An entry belongs to the operating-system user and one Cloud origin. Debug and packaged builds
/// share the app identifier, and with it the same entry; the single-instance lock keeps only one
/// of them running at a time.
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

/// Windows writes the entry itself, so it can keep the credential on this computer.
#[cfg(windows)]
#[path = "secure_store/windows_credential.rs"]
mod platform;

/// Linux goes through `keyring` to the Secret Service.
#[cfg(not(windows))]
#[path = "secure_store/secret_service.rs"]
mod platform;

impl SecureStore for KeyringStore {
    /// An unreadable entry is reported rather than treated as signed out, so a corrupted or
    /// foreign value is never silently overwritten by the next sign-in's assumptions.
    fn load(&self, server: &CloudServer) -> Result<Option<StoredSession>, StoreError> {
        let Some(raw) = platform::read(KEYRING_SERVICE, server.origin())? else {
            return Ok(None);
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
        platform::write(KEYRING_SERVICE, server.origin(), &value)
    }

    /// Deletes the entry if present.
    fn delete(&self, server: &CloudServer) -> Result<(), StoreError> {
        platform::delete(KEYRING_SERVICE, server.origin())
    }
}
