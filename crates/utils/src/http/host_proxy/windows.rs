use std::io;
use std::slice;
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, GlobalFree};
use windows_sys::Win32::Networking::WinHttp::{
    WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WinHttpGetIEProxyConfigForCurrentUser,
};

use super::{HostProxyError, StaticProxy};

/// Reads the interactive user's static configuration without running automatic discovery.
pub(super) fn read() -> Result<StaticProxy, HostProxyError> {
    let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
    // The API initializes config and allocates each non-null output with GlobalAlloc.
    let ok = unsafe { WinHttpGetIEProxyConfigForCurrentUser(&raw mut config) };
    let error = io::Error::last_os_error();
    let proxy = take_string(config.lpszProxy);
    let bypass = take_string(config.lpszProxyBypass);
    let pac = take_string(config.lpszAutoConfigUrl);
    if ok == 0 {
        return if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) {
            Ok(StaticProxy::default())
        } else {
            Err(HostProxyError::SystemRead)
        };
    }
    let mut result = StaticProxy::default();
    if let Some(proxy) = proxy.filter(|proxy| !proxy.trim().is_empty()) {
        for entry in proxy
            .split(';')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            match entry.split_once('=') {
                Some(("http", endpoint)) => result.http = Some(endpoint.to_owned()),
                Some(("https", endpoint)) => result.https = Some(endpoint.to_owned()),
                Some(_) => {}
                None => result.default = Some(entry.to_owned()),
            }
        }
    } else if config.fAutoDetect != 0 || pac.is_some_and(|pac| !pac.is_empty()) {
        return Err(HostProxyError::Automatic);
    }
    result.bypass = bypass
        .into_iter()
        .flat_map(|value| {
            value
                .split([';', ','])
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    Ok(result)
}

/// Copies a WinHTTP-owned UTF-16 string and frees the allocation.
fn take_string(value: *mut u16) -> Option<String> {
    if value.is_null() {
        return None;
    }
    // WinHTTP's outputs are NUL-terminated and owned by the caller.
    unsafe {
        let mut len = 0;
        while *value.add(len) != 0 {
            len += 1;
        }
        let owned = String::from_utf16_lossy(slice::from_raw_parts(value, len));
        GlobalFree(value.cast());
        Some(owned)
    }
}
