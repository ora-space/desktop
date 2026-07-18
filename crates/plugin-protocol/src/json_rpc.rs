use crate::{
    Frame, FrameType, JSON_SAFE_U64_MAX, StrictJsonError, deserialize_optional_non_null,
    parse_strict_json,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};

pub const JSON_RPC_VERSION: &str = "2.0";
pub const ERROR_PARSE: i32 = -32700;
pub const ERROR_INVALID_REQUEST: i32 = -32600;
pub const ERROR_METHOD_NOT_FOUND: i32 = -32601;
pub const ERROR_INVALID_PARAMS: i32 = -32602;
pub const ERROR_INTERNAL: i32 = -32603;
pub const ERROR_AGENT_BUSINESS: i32 = -32000;
pub const ERROR_SERVER_BUSY: i32 = -32010;
pub const ERROR_REQUEST_CANCELLED: i32 = -32800;

/// A bounded JSON-RPC id accepted for direction-violation diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RpcId(String);

impl RpcId {
    /// Validates the generic non-empty JSON-RPC string id boundary.
    pub fn parse(value: impl Into<String>) -> Result<Self, JsonRpcParseError> {
        let value = value.into();
        if value.is_empty() || value.len() > 128 {
            return Err(JsonRpcParseError::InvalidId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for RpcId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RpcId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// A Host-created request id with the exact `h:<JsonSafeU64>` spelling.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct HostRequestId(RpcId);

impl HostRequestId {
    /// Constructs a Host id directly from a JavaScript-safe monotonic counter.
    pub fn from_sequence(sequence: u64) -> Result<Self, JsonRpcParseError> {
        if sequence > JSON_SAFE_U64_MAX {
            return Err(JsonRpcParseError::InvalidHostRequestId);
        }
        Ok(Self(RpcId(format!("h:{sequence}"))))
    }

    /// Validates a response id received from the plugin.
    pub fn parse(value: impl Into<String>) -> Result<Self, JsonRpcParseError> {
        let value = value.into();
        let Some(sequence) = value.strip_prefix("h:") else {
            return Err(JsonRpcParseError::InvalidHostRequestId);
        };
        if sequence.is_empty()
            || (sequence.len() > 1 && sequence.starts_with('0'))
            || !sequence.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(JsonRpcParseError::InvalidHostRequestId);
        }
        let sequence = sequence
            .parse::<u64>()
            .map_err(|_| JsonRpcParseError::InvalidHostRequestId)?;
        Self::from_sequence(sequence)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl<'de> Deserialize<'de> for HostRequestId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// A parsed strict JSON-RPC envelope discriminated by JSON shape, not by the wire type byte.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonRpcEnvelope {
    Request(JsonRpcRequest),
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

/// A well-formed Request envelope; method-specific params are validated by the closed registry.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonRpcRequest {
    pub id: RpcId,
    pub method: String,
    pub params: Option<Value>,
}

/// A terminal Response envelope with exactly one of result or error.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonRpcResponse {
    Success {
        id: HostRequestId,
        result: Value,
    },
    Error {
        id: HostRequestId,
        error: JsonRpcError,
    },
}

/// A well-formed Notification envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonRpcNotification {
    pub method: String,
    pub params: Option<Value>,
}

/// The exact JSON-RPC error object shape with an optional object data payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_non_null"
    )]
    pub data: Option<Map<String, Value>>,
}

/// Classifies strict profile failures without embedding the rejected payload in diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JsonRpcParseError {
    #[error(transparent)]
    Json(#[from] StrictJsonError),
    #[error("JSON-RPC envelope must be an object and batch is unsupported")]
    EnvelopeNotObject,
    #[error("JSON-RPC version must equal 2.0")]
    InvalidVersion,
    #[error("JSON-RPC envelope has fields incompatible with its envelope shape")]
    FrameEnvelopeMismatch,
    #[error("JSON frame payload type is required before JSON-RPC parsing")]
    UnsupportedFrameType,
    #[error("JSON-RPC envelope contains an unknown top-level field")]
    UnknownField,
    #[error("JSON-RPC id must be a non-empty string no longer than 128 bytes")]
    InvalidId,
    #[error("Plugin response id must use canonical h:<JsonSafeU64> form")]
    InvalidHostRequestId,
    #[error("JSON-RPC method must be a non-empty string no longer than 256 bytes")]
    InvalidMethod,
    #[error("JSON-RPC params, when present, must be a non-null object")]
    InvalidParamsShape,
    #[error("JSON-RPC response must contain exactly one of result or error")]
    InvalidResponseShape,
    #[error("JSON-RPC error object is invalid")]
    InvalidErrorShape,
}

/// Parses one complete JSON frame according to the strict Ora JSON-RPC profile.
pub fn parse_json_rpc_frame(
    frame: &Frame,
    maximum_json_depth: usize,
) -> Result<JsonRpcEnvelope, JsonRpcParseError> {
    if frame.frame_type != FrameType::Json {
        return Err(JsonRpcParseError::UnsupportedFrameType);
    }
    let value = parse_strict_json(&frame.payload, maximum_json_depth)?;
    let object = value
        .as_object()
        .ok_or(JsonRpcParseError::EnvelopeNotObject)?;
    validate_jsonrpc_version(object)?;
    // Wire type only names the payload encoding; envelope kind comes from JSON shape.
    if object.contains_key("method") {
        if object.contains_key("id") {
            parse_request(object).map(JsonRpcEnvelope::Request)
        } else {
            parse_notification(object).map(JsonRpcEnvelope::Notification)
        }
    } else if object.contains_key("id") {
        parse_response(object).map(JsonRpcEnvelope::Response)
    } else {
        Err(JsonRpcParseError::InvalidResponseShape)
    }
}

/// Serializes one Host request envelope with exact top-level fields.
pub fn encode_json_rpc_request<P>(
    id: &HostRequestId,
    method: &str,
    params: &P,
) -> Result<Vec<u8>, serde_json::Error>
where
    P: Serialize,
{
    #[derive(Serialize)]
    struct Request<'a, P> {
        jsonrpc: &'static str,
        id: &'a HostRequestId,
        method: &'a str,
        params: &'a P,
    }

    serde_json::to_vec(&Request {
        jsonrpc: JSON_RPC_VERSION,
        id,
        method,
        params,
    })
}

/// Serializes one Host notification, omitting params for the exact exit shape when absent.
pub fn encode_json_rpc_notification<P>(
    method: &str,
    params: Option<&P>,
) -> Result<Vec<u8>, serde_json::Error>
where
    P: Serialize,
{
    #[derive(Serialize)]
    struct Notification<'a, P> {
        jsonrpc: &'static str,
        method: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        params: Option<&'a P>,
    }

    serde_json::to_vec(&Notification {
        jsonrpc: JSON_RPC_VERSION,
        method,
        params,
    })
}

/// Parses the Request profile while retaining arbitrary ids for the required fatal diagnostic.
fn parse_request(object: &Map<String, Value>) -> Result<JsonRpcRequest, JsonRpcParseError> {
    require_allowed_fields(object, &["jsonrpc", "id", "method", "params"])?;
    if object.contains_key("result") || object.contains_key("error") {
        return Err(JsonRpcParseError::FrameEnvelopeMismatch);
    }
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .ok_or(JsonRpcParseError::InvalidId)
        .and_then(RpcId::parse)?;
    let method = parse_method(object)?;
    let params = parse_optional_params(object)?;
    Ok(JsonRpcRequest { id, method, params })
}

/// Parses the Response profile and enforces Host-owned id correlation syntax.
fn parse_response(object: &Map<String, Value>) -> Result<JsonRpcResponse, JsonRpcParseError> {
    require_allowed_fields(object, &["jsonrpc", "id", "result", "error"])?;
    if object.contains_key("method") || object.contains_key("params") {
        return Err(JsonRpcParseError::FrameEnvelopeMismatch);
    }
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .ok_or(JsonRpcParseError::InvalidHostRequestId)
        .and_then(HostRequestId::parse)?;
    match (object.get("result"), object.get("error")) {
        (Some(result), None) => Ok(JsonRpcResponse::Success {
            id,
            result: result.clone(),
        }),
        (None, Some(error)) => {
            let error = serde_json::from_value::<JsonRpcError>(error.clone())
                .map_err(|_| JsonRpcParseError::InvalidErrorShape)?;
            Ok(JsonRpcResponse::Error { id, error })
        }
        _ => Err(JsonRpcParseError::InvalidResponseShape),
    }
}

/// Parses the Notification profile, leaving method-direction policy to the session router.
fn parse_notification(
    object: &Map<String, Value>,
) -> Result<JsonRpcNotification, JsonRpcParseError> {
    require_allowed_fields(object, &["jsonrpc", "method", "params"])?;
    if object.contains_key("id") || object.contains_key("result") || object.contains_key("error") {
        return Err(JsonRpcParseError::FrameEnvelopeMismatch);
    }
    let method = parse_method(object)?;
    let params = parse_optional_params(object)?;
    Ok(JsonRpcNotification { method, params })
}

/// Enforces the exact JSON-RPC version before shape-specific parsing.
fn validate_jsonrpc_version(object: &Map<String, Value>) -> Result<(), JsonRpcParseError> {
    if object.get("jsonrpc").and_then(Value::as_str) != Some(JSON_RPC_VERSION) {
        return Err(JsonRpcParseError::InvalidVersion);
    }
    Ok(())
}

/// Applies the common method string bounds.
fn parse_method(object: &Map<String, Value>) -> Result<String, JsonRpcParseError> {
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or(JsonRpcParseError::InvalidMethod)?;
    if method.is_empty() || method.len() > 256 {
        return Err(JsonRpcParseError::InvalidMethod);
    }
    Ok(method.to_string())
}

/// Rejects explicit null and non-object params in the closed v1 method registry.
fn parse_optional_params(object: &Map<String, Value>) -> Result<Option<Value>, JsonRpcParseError> {
    match object.get("params") {
        None => Ok(None),
        Some(value) if value.is_object() => Ok(Some(value.clone())),
        Some(_) => Err(JsonRpcParseError::InvalidParamsShape),
    }
}

/// Rejects unknown envelope fields before they can create shape ambiguity.
fn require_allowed_fields(
    object: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), JsonRpcParseError> {
    let allowed = allowed.iter().copied().collect::<BTreeSet<_>>();
    if object.keys().any(|key| !allowed.contains(key.as_str())) {
        return Err(JsonRpcParseError::UnknownField);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        HostRequestId, JsonRpcEnvelope, JsonRpcParseError, JsonRpcResponse,
        encode_json_rpc_request, parse_json_rpc_frame,
    };
    use crate::{Frame, FrameType};
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Parses a strict response and rejects result/error ambiguity from JSON shape alone.
    #[test]
    fn parses_strict_response_envelopes() {
        let response = Frame {
            frame_type: FrameType::Json,
            payload: br#"{"jsonrpc":"2.0","id":"h:1","result":{"ok":true}}"#.to_vec(),
        };
        assert_eq!(
            parse_json_rpc_frame(&response, 64),
            Ok(JsonRpcEnvelope::Response(JsonRpcResponse::Success {
                id: HostRequestId::from_sequence(1)
                    .unwrap_or_else(|error| panic!("expected Host id: {error}")),
                result: json!({"ok": true}),
            }))
        );

        let ambiguous = Frame {
            frame_type: FrameType::Json,
            payload: br#"{"jsonrpc":"2.0","id":"h:1","result":true,"error":{"code":1,"message":"x"}}"#
                .to_vec(),
        };
        assert_eq!(
            parse_json_rpc_frame(&ambiguous, 64),
            Err(JsonRpcParseError::InvalidResponseShape)
        );
    }

    /// Emits canonical Host request ids and compact JSON without newline framing.
    #[test]
    fn encodes_host_request() {
        let id = HostRequestId::from_sequence(7)
            .unwrap_or_else(|error| panic!("expected Host id: {error}"));
        let encoded = encode_json_rpc_request(&id, "agent.listSkills", &json!({"limit": 10}))
            .unwrap_or_else(|error| panic!("expected request encoding: {error}"));
        assert_eq!(
            encoded,
            br#"{"jsonrpc":"2.0","id":"h:7","method":"agent.listSkills","params":{"limit":10}}"#
                .to_vec()
        );
    }
}
