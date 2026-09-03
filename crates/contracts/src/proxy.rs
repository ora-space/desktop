use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes an optional host-level network proxy used by marketplace traffic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct ProxySettings {
    /// Proxy hostname or address without a scheme.
    pub host: String,
    /// Proxy TCP port.
    pub port: u16,
    /// Optional HTTP Basic username for the proxy.
    pub username: Option<String>,
    /// Optional HTTP Basic password for the proxy.
    pub password: Option<String>,
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
    pub settings: Option<ProxySettings>,
}

/// Requests replacing the configured network proxy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct SetProxySettingsRequest {
    pub settings: ProxySettings,
}

/// Returns the authoritative network proxy after a save.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct SetProxySettingsResponse {
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
    pub settings: Option<ProxySettings>,
}

/// Requests probing one URL through an explicit proxy configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct CheckProxySettingsRequest {
    /// Absolute HTTP(S) URL to fetch through the supplied proxy.
    pub url: String,
    /// Proxy configuration to probe with, independent of whatever is currently persisted.
    pub settings: ProxySettings,
}

/// Reports whether the supplied proxy could reach the requested URL.
///
/// Any HTTP response, including 4xx and 5xx, means the proxy path reached a host. Transport
/// failures such as timeouts, refused connections, and invalid URLs are `unreachable`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "proxy.ts")]
pub enum CheckProxySettingsResponse {
    Reachable { status: u16 },
    Unreachable { message: String },
}

/// Exports the complete network-proxy DTO family into one TypeScript module.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
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
                settings: settings.clone(),
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
            serde_json::to_value(GetProxySettingsResponse { settings: None }).unwrap(),
            json!({ "settings": null })
        );
        assert_eq!(
            serde_json::to_value(ClearProxySettingsRequest {}).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(ClearProxySettingsResponse { settings: None }).unwrap(),
            json!({ "settings": null })
        );
        assert_eq!(
            serde_json::to_value(CheckProxySettingsRequest {
                url: "https://example.com/".to_string(),
                settings,
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
            serde_json::to_value(CheckProxySettingsResponse::Reachable { status: 200 }).unwrap(),
            json!({ "outcome": "reachable", "status": 200 })
        );
        assert_eq!(
            serde_json::to_value(CheckProxySettingsResponse::Unreachable {
                message: "connection refused".to_string(),
            })
            .unwrap(),
            json!({ "outcome": "unreachable", "message": "connection refused" })
        );
    }
}
