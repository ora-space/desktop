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
    match settings.string("mode").as_str() {
        "none" => Ok(Some(StaticProxy::default())),
        "auto" => Err(HostProxyError::Automatic),
        "manual" => Ok(Some(StaticProxy {
            http: endpoint(&settings.child("http"), "http")?,
            https: endpoint(&settings.child("https"), "http")?,
            default: endpoint(&settings.child("socks"), "socks5h")?,
            bypass: settings
                .strv("ignore-hosts")
                .iter()
                .map(ToString::to_string)
                .collect(),
        })),
        _ => Err(HostProxyError::SystemRead),
    }
}

/// Reads the host and port of one static proxy protocol.
fn endpoint(settings: &gio::Settings, scheme: &str) -> Result<Option<String>, HostProxyError> {
    let host = settings.string("host");
    if host.is_empty() {
        return Ok(None);
    }
    let port = u16::try_from(settings.int("port"))
        .ok()
        .filter(|port| *port != 0)
        .ok_or(HostProxyError::InvalidEndpoint)?;
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
