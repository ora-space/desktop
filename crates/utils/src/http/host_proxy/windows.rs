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
    parse_static(proxy.as_deref(), bypass.as_deref(), pac.as_deref())
}

/// Interprets only explicit proxy endpoints. The default WPAD flag needs no route of its own.
fn parse_static(
    proxy: Option<&str>,
    bypass: Option<&str>,
    pac: Option<&str>,
) -> Result<StaticProxy, HostProxyError> {
    let mut result = StaticProxy::default();
    if let Some(proxy) = proxy.filter(|proxy| !proxy.trim().is_empty()) {
        for entry in proxy
            .split(';')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            match entry
                .split_once('=')
                .map(|(scheme, endpoint)| (scheme.trim(), endpoint.trim()))
            {
                Some((scheme, endpoint)) if scheme.eq_ignore_ascii_case("http") => {
                    result.http = Some(endpoint.to_owned());
                }
                Some((scheme, endpoint)) if scheme.eq_ignore_ascii_case("https") => {
                    result.https = Some(endpoint.to_owned());
                }
                Some(_) => {}
                None => result.default = Some(entry.to_owned()),
            }
        }
        if result.http.is_none() && result.https.is_none() && result.default.is_none() {
            return Err(HostProxyError::InvalidEndpoint);
        }
    } else if pac.is_some_and(|pac| !pac.trim().is_empty()) {
        return Err(HostProxyError::Automatic);
    }
    result.bypass = bypass
        .unwrap_or_default()
        .split([';', ','])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HostProxy, ProxyOrigin};

    /// Stock Windows auto-detection supplies no static endpoint or PAC URL.
    #[test]
    fn host_proxy_windows_distinguishes_static_direct_pac_and_unsupported() {
        let url = "https://example.test/".parse().unwrap();
        let use_settings = |settings| HostProxy {
            settings,
            origin: ProxyOrigin::System,
        };
        assert_eq!(
            use_settings(parse_static(None, None, None))
                .route(&url)
                .unwrap()
                .origin,
            ProxyOrigin::Direct
        );
        assert!(matches!(
            use_settings(parse_static(None, None, Some("http://pac.test/config.pac"))).route(&url),
            Err(HostProxyError::Automatic)
        ));
        assert!(matches!(
            use_settings(parse_static(Some("socks=proxy.test:1080"), None, None)).route(&url),
            Err(HostProxyError::InvalidEndpoint)
        ));
        let policy = use_settings(parse_static(
            Some("http=proxy.test:8080;HTTPS=secure.test:3128"),
            Some("*.corp.test;<local>"),
            Some("http://pac.test/config.pac"),
        ));
        assert_eq!(
            policy.route(&url).unwrap().endpoint.unwrap().as_str(),
            "http://secure.test:3128/"
        );
        for host in ["app.corp.test", "intranet", "localhost"] {
            let url = format!("https://{host}/").parse().unwrap();
            assert_eq!(policy.route(&url).unwrap().origin, ProxyOrigin::Direct);
        }
    }
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
