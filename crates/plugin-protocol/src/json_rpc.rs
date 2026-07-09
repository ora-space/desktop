use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Carries the named parameters for the first plugin `add` capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin-protocol.ts")]
pub struct PluginAddParams {
    #[ts(type = "number")]
    pub a: i64,
    #[ts(type = "number")]
    pub b: i64,
}

/// Carries one JSON-RPC request sent from the plugin manager to the plugin SDK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin-protocol.ts")]
pub struct PluginJsonRpcRequest {
    pub jsonrpc: String,
    pub id: String,
    pub method: String,
    pub params: PluginAddParams,
}

/// Carries one successful JSON-RPC response emitted by the plugin SDK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin-protocol.ts")]
pub struct PluginJsonRpcSuccessResponse {
    pub jsonrpc: String,
    pub id: String,
    #[ts(type = "number")]
    pub result: i64,
}

/// Carries JSON-RPC error details emitted by the plugin SDK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin-protocol.ts")]
pub struct PluginJsonRpcError {
    #[ts(type = "number")]
    pub code: i64,
    pub message: String,
}

/// Carries one failed JSON-RPC response emitted by the plugin SDK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin-protocol.ts")]
pub struct PluginJsonRpcErrorResponse {
    pub jsonrpc: String,
    pub id: String,
    pub error: PluginJsonRpcError,
}

#[cfg(test)]
mod tests {
    use super::{
        PluginAddParams, PluginJsonRpcError, PluginJsonRpcErrorResponse, PluginJsonRpcRequest,
        PluginJsonRpcSuccessResponse,
    };
    use pretty_assertions::assert_eq;
    use serde::Serialize;
    use serde_json::{Value, json};

    /// Verifies plugin JSON-RPC protocol DTOs serialize to the newline-framed payload shapes.
    #[test]
    fn serializes_plugin_json_rpc_protocol() {
        assert_serialized_json(
            &PluginJsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: "1".to_string(),
                method: "add".to_string(),
                params: PluginAddParams { a: 1, b: 2 },
            },
            json!({
                "jsonrpc": "2.0",
                "id": "1",
                "method": "add",
                "params": {
                    "a": 1,
                    "b": 2,
                },
            }),
        );
        assert_serialized_json(
            &PluginJsonRpcSuccessResponse {
                jsonrpc: "2.0".to_string(),
                id: "1".to_string(),
                result: 3,
            },
            json!({
                "jsonrpc": "2.0",
                "id": "1",
                "result": 3,
            }),
        );
        assert_serialized_json(
            &PluginJsonRpcErrorResponse {
                jsonrpc: "2.0".to_string(),
                id: "1".to_string(),
                error: PluginJsonRpcError {
                    code: -32601,
                    message: "missing method".to_string(),
                },
            },
            json!({
                "jsonrpc": "2.0",
                "id": "1",
                "error": {
                    "code": -32601,
                    "message": "missing method",
                },
            }),
        );
    }

    /// Serializes one value and compares the exact JSON payload.
    fn assert_serialized_json(value: &impl Serialize, expected: Value) {
        let serialized = serde_json::to_value(value)
            .unwrap_or_else(|error| panic!("expected JSON serialization to succeed: {error}"));

        assert_eq!(serialized, expected);
    }
}
