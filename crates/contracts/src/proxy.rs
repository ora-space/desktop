use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Requests the user's saved proxy probe target.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct GetProxyCheckUrlRequest {}

/// Explicitly confirms a new HTTP(S) proxy probe target.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct SetProxyCheckUrlRequest {
    pub url: String,
}

/// Returns the authoritative saved target, or the default only when no preference exists.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct ProxyCheckUrlResponse {
    /// The configured release manifest URL, when the host supplies one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub update_url: Option<String>,
    pub url: String,
}

/// Describes a manual endpoint for the Desktop updater and opted-in marketplace traffic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct ProxySettings {
    /// Proxy hostname or address, optionally prefixed with its transport scheme.
    pub host: String,
    /// Proxy TCP port.
    pub port: u16,
    /// Optional HTTP Basic username for the proxy.
    pub username: Option<String>,
    /// Optional HTTP Basic password for the proxy.
    pub password: Option<String>,
}

/// Selects how Desktop chooses a network proxy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "proxy.ts")]
pub enum ProxyMode {
    #[default]
    System,
    Manual,
    Direct,
}

/// Identifies the route a connectivity probe actually used.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "proxy.ts")]
pub enum ProxyRoute {
    System,
    Manual,
    Environment,
    Direct,
}

/// Requests the configured network proxy without additional parameters.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct GetProxySettingsRequest {}

/// Returns the configured network proxy.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct GetProxySettingsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mode: Option<ProxyMode>,
    pub settings: Option<ProxySettings>,
}

/// Requests replacing the configured network proxy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct SetProxySettingsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mode: Option<ProxyMode>,
    pub settings: Option<ProxySettings>,
}

/// Returns the authoritative network proxy after a save.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct SetProxySettingsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mode: Option<ProxyMode>,
    pub settings: Option<ProxySettings>,
}

/// Requests deleting the configured network proxy.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct ClearProxySettingsRequest {}

/// Confirms the network proxy has been cleared.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct ClearProxySettingsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mode: Option<ProxyMode>,
    pub settings: Option<ProxySettings>,
}

/// Requests probing one URL through an explicit proxy configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct CheckProxySettingsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mode: Option<ProxyMode>,
    /// Absolute HTTP(S) URL to fetch through the supplied proxy.
    pub url: String,
    /// Proxy configuration to probe with, independent of whatever is currently persisted.
    pub settings: Option<ProxySettings>,
}

/// Reports whether the supplied proxy could reach the requested URL.
///
/// A final 2xx response is reachable. Other final statuses are HTTP errors, except for 407.
/// Proxy authentication, transport and proxy-resolution failures are `unreachable`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "proxy.ts")]
pub enum CheckProxySettingsResponse {
    Reachable {
        status: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        route: Option<ProxyRoute>,
    },
    HttpError {
        status: u16,
    },
    Unreachable {
        message: String,
    },
}

/// Exports the complete network-proxy DTO family into one TypeScript module.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    GetProxyCheckUrlRequest::export(config)?;
    SetProxyCheckUrlRequest::export(config)?;
    ProxyCheckUrlResponse::export(config)?;
    ProxyMode::export(config)?;
    ProxyRoute::export(config)?;
    ProxySettings::export(config)?;
    GetProxySettingsRequest::export(config)?;
    GetProxySettingsResponse::export(config)?;
    SetProxySettingsRequest::export(config)?;
    SetProxySettingsResponse::export(config)?;
    ClearProxySettingsRequest::export(config)?;
    ClearProxySettingsResponse::export(config)?;
    CheckProxySettingsRequest::export(config)?;
    CheckProxySettingsResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::{
        CheckProxySettingsRequest, CheckProxySettingsResponse, ClearProxySettingsRequest,
        ClearProxySettingsResponse, GetProxySettingsRequest, GetProxySettingsResponse,
        ProxySettings, SetProxySettingsRequest,
    };

    #[test]
    fn serializes_proxy_settings_contracts() {
        let settings = ProxySettings {
            host: "127.0.0.1".to_string(),
            port: 7890,
            username: None,
            password: None,
        };
        assert_eq!(
            serde_json::to_value(GetProxySettingsRequest {}).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(SetProxySettingsRequest {
                settings: Some(settings.clone()),
                mode: None,
            })
            .unwrap(),
            json!({
                "settings": {
                    "host": "127.0.0.1",
                    "port": 7890,
                    "username": null,
                    "password": null
                }
            })
        );
        assert_eq!(
            serde_json::to_value(GetProxySettingsResponse {
                settings: None,
                mode: None
            })
            .unwrap(),
            json!({ "settings": null })
        );
        assert_eq!(
            serde_json::to_value(ClearProxySettingsRequest {}).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(ClearProxySettingsResponse {
                settings: None,
                mode: None
            })
            .unwrap(),
            json!({ "settings": null })
        );
        assert_eq!(
            serde_json::to_value(CheckProxySettingsRequest {
                url: "https://example.com/".to_string(),
                settings: Some(settings),
                mode: None,
            })
            .unwrap(),
            json!({
                "url": "https://example.com/",
                "settings": {
                    "host": "127.0.0.1",
                    "port": 7890,
                    "username": null,
                    "password": null
                }
            })
        );
        assert_eq!(
            serde_json::to_value(CheckProxySettingsResponse::Reachable {
                status: 200,
                route: None
            })
            .unwrap(),
            json!({ "outcome": "reachable", "status": 200 })
        );
        assert_eq!(
            serde_json::to_value(CheckProxySettingsResponse::Unreachable {
                message: "connection refused".to_string(),
            })
            .unwrap(),
            json!({ "outcome": "unreachable", "message": "connection refused" })
        );
        assert_eq!(
            serde_json::to_value(CheckProxySettingsResponse::HttpError { status: 404 }).unwrap(),
            json!({ "outcome": "http_error", "status": 404 })
        );
    }
}
