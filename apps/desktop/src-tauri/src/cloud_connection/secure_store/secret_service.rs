//! The Linux Secret Service, through `keyring`: one item per service and origin.

use super::StoreError;

/// Opens the item for one origin.
fn entry(service: &str, origin: &str) -> Result<keyring::Entry, StoreError> {
    keyring::Entry::new(service, origin).map_err(store_error)
}

/// Keeps the platform's explanation without anything that could be the secret.
fn store_error(error: keyring::Error) -> StoreError {
    StoreError(error.to_string())
}

/// Reads the stored value, or `None` when there is no item.
pub(super) fn read(service: &str, origin: &str) -> Result<Option<String>, StoreError> {
    match entry(service, origin)?.get_password() {
        Ok(raw) => Ok(Some(raw)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(store_error(error)),
    }
}

/// Replaces the stored value.
pub(super) fn write(service: &str, origin: &str, value: &str) -> Result<(), StoreError> {
    entry(service, origin)?
        .set_password(value)
        .map_err(store_error)
}

/// Removes the item; removing nothing is not an error.
pub(super) fn delete(service: &str, origin: &str) -> Result<(), StoreError> {
    match entry(service, origin)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(store_error(error)),
    }
}
