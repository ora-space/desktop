//! Windows Credential Manager, written directly so the entry stays on this computer.
//!
//! `keyring` writes every credential with `CRED_PERSIST_ENTERPRISE`, which roams with a roaming
//! user profile to other computers in the domain. A desktop token belongs to one installation:
//! signing out "on this computer" must not leave a working copy elsewhere, and a copy that roamed
//! would be used from a computer that never signed in. The entry is therefore a generic
//! credential with `CRED_PERSIST_LOCAL_MACHINE`, visible only to this user on this computer.
//!
//! The target name stays `<origin>.<service>` (what `keyring` used) and the blob stays the value
//! encoded as UTF-16LE, so Credential Manager shows the same item and a value written by an
//! earlier build reads back.

use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError};
use windows_sys::Win32::Security::Credentials::{
    CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW,
    CredDeleteW, CredFree, CredReadW, CredWriteW,
};

use super::StoreError;

/// The credential's target name, which Windows uses as its only key.
fn target_name(service: &str, origin: &str) -> Vec<u16> {
    wide(&format!("{origin}.{service}"))
}

/// A NUL-terminated UTF-16 string.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Describes the last Windows error without anything that could be the secret.
fn last_error(operation: &str) -> StoreError {
    // SAFETY: GetLastError only reads the calling thread's last-error value.
    let code = unsafe { GetLastError() };
    StoreError(format!(
        "Windows Credential Manager {operation} failed ({})",
        std::io::Error::from_raw_os_error(code as i32)
    ))
}

/// Reads the stored value, or `None` when there is no credential.
pub(super) fn read(service: &str, origin: &str) -> Result<Option<String>, StoreError> {
    let target = target_name(service, origin);
    let mut credential: *mut CREDENTIALW = ptr::null_mut();
    // SAFETY: `target` is NUL-terminated and outlives the call; on success Windows allocates the
    // credential and stores its address in `credential`, which is freed below with CredFree.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
        // SAFETY: as in `last_error`.
        if unsafe { GetLastError() } == ERROR_NOT_FOUND {
            return Ok(None);
        }
        return Err(last_error("read"));
    }
    // SAFETY: CredReadW succeeded, so `credential` points at a valid CREDENTIALW whose blob is
    // `CredentialBlobSize` bytes long (or null when empty).
    let blob = unsafe {
        let record = &*credential;
        let size = record.CredentialBlobSize as usize;
        if size == 0 || record.CredentialBlob.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(record.CredentialBlob, size).to_vec()
        }
    };
    // SAFETY: `credential` came from CredReadW and is freed exactly once.
    unsafe { CredFree(credential.cast()) };
    decode(&blob).map(Some)
}

/// Replaces the stored value.
pub(super) fn write(service: &str, origin: &str, value: &str) -> Result<(), StoreError> {
    let mut target = target_name(service, origin);
    let mut user = wide(origin);
    let mut blob = encode(value);
    if blob.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
        return Err(StoreError(
            "the Cloud credential is too large for Windows Credential Manager".to_owned(),
        ));
    }
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: user.as_mut_ptr(),
        ..Default::default()
    };
    // SAFETY: every pointer in `credential` refers to a buffer that lives until after the call;
    // CredWriteW copies what it keeps.
    if unsafe { CredWriteW(&credential, 0) } == 0 {
        return Err(last_error("write"));
    }
    Ok(())
}

/// Removes the credential; removing nothing is not an error.
pub(super) fn delete(service: &str, origin: &str) -> Result<(), StoreError> {
    let target = target_name(service, origin);
    // SAFETY: `target` is NUL-terminated and outlives the call.
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
        // SAFETY: as in `last_error`.
        if unsafe { GetLastError() } == ERROR_NOT_FOUND {
            return Ok(());
        }
        return Err(last_error("delete"));
    }
    Ok(())
}

/// The value as UTF-16LE bytes, the encoding Credential Manager tools display.
fn encode(value: &str) -> Vec<u8> {
    value.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// Reads a UTF-16LE blob back into the value.
fn decode(blob: &[u8]) -> Result<String, StoreError> {
    let unreadable = || StoreError("the stored Cloud credential is unreadable".to_owned());
    if !blob.len().is_multiple_of(2) {
        return Err(unreadable());
    }
    let units: Vec<u16> = blob
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    String::from_utf16(&units).map_err(|_| unreadable())
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    /// The blob round-trips, and an odd-length blob is unreadable rather than truncated.
    #[test]
    fn blob_round_trips_as_utf16() {
        let value = r#"{"token":"ora_dt_x","accountId":"u","displayName":"张三"}"#;
        assert_eq!(decode(&encode(value)).expect("decode"), value);
        assert!(decode(&[0x41]).is_err());
    }
}
