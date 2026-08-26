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

/// Requests replacing the configured network proxy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct SetProxySettingsRequest {
    pub settings: ProxySettings,
}

/// Returns the configured network proxy.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct GetProxySettingsResponse {
    pub settings: Option<ProxySettings>,
}

/// Returns the authoritative network proxy after a save.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "proxy.ts")]
pub struct SetProxySettingsResponse {
    pub settings: Option<ProxySettings>,
}

/// Exports the complete network-proxy DTO family into one TypeScript module.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    ProxySettings::export(config)?;
    GetProxySettingsRequest::export(config)?;
    SetProxySettingsRequest::export(config)?;
    GetProxySettingsResponse::export(config)?;
    SetProxySettingsResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::{
        GetProxySettingsRequest, GetProxySettingsResponse, ProxySettings, SetProxySettingsRequest,
    };

    #[test]
    fn serializes_proxy_settings_contracts() {
        assert_eq!(
            serde_json::to_value(GetProxySettingsRequest {}).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(SetProxySettingsRequest {
                settings: ProxySettings {
                    host: "127.0.0.1".to_string(),
                    port: 7890,
                    username: None,
                    password: None,
                },
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
    }
}
