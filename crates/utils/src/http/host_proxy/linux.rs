use gio::prelude::*;

use super::{HostProxyError, StaticProxy};

/// Reads GNOME's static GIO settings directly. Generic resolver plugins may evaluate PAC during
/// lookup, so only the readable static settings backend is used here.
pub(super) fn read() -> Result<Option<StaticProxy>, HostProxyError> {
    let is_gnome = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .any(|desktop| desktop.eq_ignore_ascii_case("GNOME"));
    if !is_gnome {
        return Ok(None);
    }
    let Some(schema) = gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup("org.gnome.system.proxy", true))
    else {
        return Ok(None);
    };
    let settings = gio::Settings::new_full(&schema, gio::SettingsBackend::NONE, None);
    read_settings(&settings).map(Some)
}

/// Applies the static GNOME scheme precedence without calling the PAC evaluator.
fn read_settings(settings: &gio::Settings) -> Result<StaticProxy, HostProxyError> {
    match settings.string("mode").as_str() {
        "none" => Ok(StaticProxy::default()),
        "auto" => Err(HostProxyError::Automatic),
        "manual" => {
            let http = endpoint(&settings.child("http"), "http")?;
            // GNOME uses the HTTP proxy for HTTPS unless an HTTPS proxy is configured.
            let https = endpoint(&settings.child("https"), "http")?.or_else(|| http.clone());
            Ok(StaticProxy {
                http,
                https,
                default: endpoint(&settings.child("socks"), "socks5h")?,
                bypass: settings
                    .strv("ignore-hosts")
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            })
        }
        _ => Err(HostProxyError::SystemRead),
    }
}

/// Reads the host and port of one static proxy protocol.
fn endpoint(settings: &gio::Settings, scheme: &str) -> Result<Option<String>, HostProxyError> {
    let host = settings.string("host");
    let port = settings.int("port");
    if host.is_empty() || port == 0 {
        return Ok(None);
    }
    let port = u16::try_from(port).map_err(|_| HostProxyError::InvalidEndpoint)?;
    let mut endpoint = super::proxy_endpoint(&format!("{scheme}://{host}:{port}"))?;
    if scheme == "http"
        && settings
            .settings_schema()
            .is_some_and(|schema| schema.has_key("use-authentication"))
        && settings.boolean("use-authentication")
    {
        endpoint
            .set_username(&settings.string("authentication-user"))
            .map_err(|_| HostProxyError::InvalidEndpoint)?;
        endpoint
            .set_password(Some(&settings.string("authentication-password")))
            .map_err(|_| HostProxyError::InvalidEndpoint)?;
    }
    Ok(Some(endpoint.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Exercises HTTP fallback, explicit HTTPS precedence and disabled endpoints through GSettings.
    #[test]
    fn host_proxy_gnome_https_follows_http_until_an_override_is_enabled() {
        let schema = gio::SettingsSchemaSource::default()
            .and_then(|source| source.lookup("org.gnome.system.proxy", true))
            .expect("install gsettings-desktop-schemas for the desktop proxy tests");
        let backend = gio::memory_settings_backend_new();
        let settings = gio::Settings::new_full(&schema, Some(&backend), None);
        settings.set_string("mode", "manual").expect("mode");
        let http = settings.child("http");
        http.set_string("host", "http-proxy.example")
            .expect("HTTP host");
        http.set_int("port", 8080).expect("HTTP port");
        let https = settings.child("https");
        https
            .set_string("host", "https-proxy.example")
            .expect("HTTPS host");
        // GNOME treats a retained hostname with port zero as a disabled protocol override.
        https.set_int("port", 0).expect("disabled HTTPS port");
        assert_eq!(
            read_settings(&settings)
                .expect("HTTP fallback")
                .https
                .as_deref(),
            Some("http://http-proxy.example:8080/")
        );

        https.set_int("port", 8443).expect("HTTPS port");
        assert_eq!(
            read_settings(&settings)
                .expect("explicit HTTPS")
                .https
                .as_deref(),
            Some("http://https-proxy.example:8443/")
        );
    }
}
