//! Optional MCP Configuration Capability published during `ora/register`.
//!
//! Handshake must stay additive: older plugins omit the field, and a malformed declaration only
//! disables MCP materialization. The runtime therefore never fails registration because this
//! payload is missing, unknown at the top level, or internally invalid.

use std::collections::BTreeSet;
use std::fmt::{self, Display, Formatter};
use std::num::NonZeroU32;

use serde_json::{Map, Value, json};

/// JSON-RPC method that must be registered together with a valid MCP configuration capability.
pub const CONFIGURE_WORKSPACE_METHOD: &str = "agent/configureWorkspace";

/// Protocol v1 is the only Host-supported MCP configuration snapshot version.
pub const MCP_CONFIGURATION_PROTOCOL_V1: u32 = 1;

/// Positive protocol version advertised by an Agent plugin.
///
/// Zero is unrepresentable because the wire contract requires a positive integer; unsupported
/// positive versions are still constructible so Host negotiation can classify them without
/// collapsing the value into a boolean flag.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct McpConfigurationProtocolVersion(NonZeroU32);

impl McpConfigurationProtocolVersion {
    /// Protocol v1, the only version this Host materializes.
    pub const V1: Self = Self(NonZeroU32::MIN);

    /// Accepts a positive integer from the registration wire.
    pub fn new(value: u32) -> Result<Self, McpConfigurationCapabilityIssue> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or(McpConfigurationCapabilityIssue::ProtocolVersionNotPositiveInteger)
    }

    /// Returns the integer advertised on the wire.
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

/// Transport kinds an Agent plugin may materialize in protocol v1.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum McpTransportKind {
    Http,
    Stdio,
}

impl McpTransportKind {
    /// Parses one wire token; unknown tokens invalidate the capability rather than the handshake.
    pub fn parse(value: &str) -> Result<Self, McpConfigurationCapabilityIssue> {
        match value {
            "http" => Ok(Self::Http),
            "stdio" => Ok(Self::Stdio),
            other => Err(McpConfigurationCapabilityIssue::UnknownTransport(
                other.to_string(),
            )),
        }
    }

    /// Returns the canonical wire token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Stdio => "stdio",
        }
    }
}

/// Non-empty unique set of supported transports.
///
/// Construction is the only way to obtain a value, so empty and duplicate sets cannot leak into
/// Host snapshot filtering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpTransportSet {
    kinds: BTreeSet<McpTransportKind>,
}

impl McpTransportSet {
    /// Accepts a unique, non-empty transport list from registration or SDK input.
    pub fn new(
        kinds: impl IntoIterator<Item = McpTransportKind>,
    ) -> Result<Self, McpConfigurationCapabilityIssue> {
        let mut unique = BTreeSet::new();
        for kind in kinds {
            if !unique.insert(kind) {
                return Err(McpConfigurationCapabilityIssue::DuplicateTransport(
                    kind.as_str().to_string(),
                ));
            }
        }
        if unique.is_empty() {
            return Err(McpConfigurationCapabilityIssue::TransportsEmpty);
        }
        Ok(Self { kinds: unique })
    }

    /// Returns whether this Agent can materialize `kind`.
    pub fn supports(&self, kind: McpTransportKind) -> bool {
        self.kinds.contains(&kind)
    }

    /// Iterates supported kinds in canonical order so digest bytes stay stable.
    pub fn iter(&self) -> impl Iterator<Item = McpTransportKind> + '_ {
        self.kinds.iter().copied()
    }
}

/// Coordination mode an Agent uses around MCP materialization.
///
/// Protocol v1 only negotiates wait-for-idle-and-restart so Session turns cannot observe a
/// half-written configuration document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpCoordinationMode {
    WaitForIdleAndRestart,
}

impl McpCoordinationMode {
    /// Parses the only legal protocol v1 coordination token.
    pub fn parse(value: &str) -> Result<Self, McpConfigurationCapabilityIssue> {
        match value {
            "wait_for_idle_and_restart" => Ok(Self::WaitForIdleAndRestart),
            other => Err(McpConfigurationCapabilityIssue::UnknownCoordination(
                other.to_string(),
            )),
        }
    }

    /// Returns the canonical wire token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WaitForIdleAndRestart => "wait_for_idle_and_restart",
        }
    }
}

/// Structurally valid MCP configuration capability declared by an Agent plugin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpConfigurationCapability {
    protocol_version: McpConfigurationProtocolVersion,
    transports: McpTransportSet,
    coordination: McpCoordinationMode,
}

impl McpConfigurationCapability {
    /// Builds a capability after the caller has already enforced uniqueness and presence.
    pub fn new(
        protocol_version: McpConfigurationProtocolVersion,
        transports: McpTransportSet,
        coordination: McpCoordinationMode,
    ) -> Self {
        Self {
            protocol_version,
            transports,
            coordination,
        }
    }

    /// Returns the advertised protocol version.
    pub fn protocol_version(&self) -> McpConfigurationProtocolVersion {
        self.protocol_version
    }

    /// Returns the supported transport set used to filter snapshot entries.
    pub fn transports(&self) -> &McpTransportSet {
        &self.transports
    }

    /// Returns the coordination mode the Host must honor before calling configure.
    pub fn coordination(&self) -> McpCoordinationMode {
        self.coordination
    }

    /// Canonical JSON bytes hashed into an Agent Capability Revision.
    ///
    /// Key and transport order are fixed so equivalent declarations produce the same digest even
    /// when the plugin authored a different JSON key sequence.
    pub fn canonical_declaration_bytes(&self) -> Vec<u8> {
        let transports = self
            .transports
            .iter()
            .map(McpTransportKind::as_str)
            .collect::<Vec<_>>();
        serde_json::to_vec(&json!({
            "coordination": self.coordination.as_str(),
            "protocolVersion": self.protocol_version.get(),
            "transports": transports,
        }))
        .unwrap_or_else(|_| {
            // serde_json serialization of these primitive values cannot fail; keep the Host
            // digest path total rather than introducing a fallible capability type.
            b"{}".to_vec()
        })
    }
}

/// Why a present `mcpConfiguration` object cannot be used for materialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum McpConfigurationCapabilityIssue {
    NotAnObject,
    UnknownField(String),
    ProtocolVersionMissing,
    ProtocolVersionNotPositiveInteger,
    ProtocolVersionUnsupported { actual: u32 },
    TransportsMissing,
    TransportsEmpty,
    TransportsNotAnArray,
    DuplicateTransport(String),
    UnknownTransport(String),
    CoordinationMissing,
    UnknownCoordination(String),
}

impl McpConfigurationCapabilityIssue {
    /// Stable public error code the Host surfaces without copying capability payload bytes.
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::ProtocolVersionUnsupported { .. } => "mcp_capability_version_unsupported",
            Self::NotAnObject
            | Self::UnknownField(_)
            | Self::ProtocolVersionMissing
            | Self::ProtocolVersionNotPositiveInteger
            | Self::TransportsMissing
            | Self::TransportsEmpty
            | Self::TransportsNotAnArray
            | Self::DuplicateTransport(_)
            | Self::UnknownTransport(_)
            | Self::CoordinationMissing
            | Self::UnknownCoordination(_) => "mcp_capability_invalid",
        }
    }
}

impl Display for McpConfigurationCapabilityIssue {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnObject => formatter.write_str("mcpConfiguration must be an object"),
            Self::UnknownField(field) => {
                write!(formatter, "mcpConfiguration contains unknown field {field}")
            }
            Self::ProtocolVersionMissing => {
                formatter.write_str("mcpConfiguration is missing protocolVersion")
            }
            Self::ProtocolVersionNotPositiveInteger => {
                formatter.write_str("mcpConfiguration protocolVersion must be a positive integer")
            }
            Self::ProtocolVersionUnsupported { actual } => {
                write!(
                    formatter,
                    "mcpConfiguration protocolVersion {actual} is unsupported"
                )
            }
            Self::TransportsMissing => {
                formatter.write_str("mcpConfiguration is missing transports")
            }
            Self::TransportsEmpty => {
                formatter.write_str("mcpConfiguration transports must not be empty")
            }
            Self::TransportsNotAnArray => {
                formatter.write_str("mcpConfiguration transports must be an array")
            }
            Self::DuplicateTransport(transport) => {
                write!(
                    formatter,
                    "mcpConfiguration transports contains duplicate {transport}"
                )
            }
            Self::UnknownTransport(transport) => {
                write!(
                    formatter,
                    "mcpConfiguration transports contains unknown {transport}"
                )
            }
            Self::CoordinationMissing => {
                formatter.write_str("mcpConfiguration is missing coordination")
            }
            Self::UnknownCoordination(mode) => {
                write!(formatter, "mcpConfiguration coordination {mode} is unknown")
            }
        }
    }
}

/// Wire-level MCP configuration capability as published during `ora/register`.
///
/// `Absent` is the older-plugin case. `Invalid` keeps the process usable for conversation while
/// disabling MCP materialization. `Declared` is structurally valid and still subject to Host
/// pairing with `agent/configureWorkspace`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum McpConfigurationRegistration {
    #[default]
    Absent,
    Invalid(McpConfigurationCapabilityIssue),
    Declared(McpConfigurationCapability),
}

impl McpConfigurationRegistration {
    /// Canonical bytes hashed into an Agent Capability Revision for any registration outcome.
    ///
    /// Invalid declarations still produce a stable digest so repairing a malformed capability is
    /// observed as a real capability change rather than as a silent no-op.
    pub fn canonical_declaration_bytes(&self) -> Vec<u8> {
        match self {
            Self::Absent => b"null".to_vec(),
            Self::Invalid(issue) => {
                // Hash the specific issue, not only the public error code, so repairing one
                // malformed payload into a different malformed payload is still a real change.
                format!("invalid:{issue}").into_bytes()
            }
            Self::Declared(capability) => capability.canonical_declaration_bytes(),
        }
    }
}

/// Reads the optional `mcpConfiguration` object without failing the plugin handshake.
pub fn parse_mcp_configuration(params: Option<&Value>) -> McpConfigurationRegistration {
    let Some(value) = params.and_then(|params| params.get("mcpConfiguration")) else {
        return McpConfigurationRegistration::Absent;
    };
    parse_mcp_configuration_value(value)
}

/// Parses one capability JSON value using the same rules Host and compatibility fixtures share.
fn parse_mcp_configuration_value(value: &Value) -> McpConfigurationRegistration {
    let Some(object) = value.as_object() else {
        return McpConfigurationRegistration::Invalid(McpConfigurationCapabilityIssue::NotAnObject);
    };
    match parse_capability_object(object) {
        Ok(capability) => McpConfigurationRegistration::Declared(capability),
        Err(issue) => McpConfigurationRegistration::Invalid(issue),
    }
}

/// Enforces the closed protocol v1 object shape, including unknown-field rejection.
///
/// Unknown keys are rejected here rather than ignored so a plugin cannot silently advertise a
/// native schema or extra coordination the Host would never honor.
fn parse_capability_object(
    object: &Map<String, Value>,
) -> Result<McpConfigurationCapability, McpConfigurationCapabilityIssue> {
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "protocolVersion" | "transports" | "coordination"
        ) {
            return Err(McpConfigurationCapabilityIssue::UnknownField(key.clone()));
        }
    }
    let protocol_version = parse_protocol_version(object.get("protocolVersion"))?;
    if protocol_version.get() != MCP_CONFIGURATION_PROTOCOL_V1 {
        return Err(
            McpConfigurationCapabilityIssue::ProtocolVersionUnsupported {
                actual: protocol_version.get(),
            },
        );
    }
    let transports = parse_transports(object.get("transports"))?;
    let coordination = parse_coordination(object.get("coordination"))?;
    Ok(McpConfigurationCapability::new(
        protocol_version,
        transports,
        coordination,
    ))
}

/// Requires a JSON number that can become a positive protocol version.
///
/// Zero and non-integers are the same invalid class because the wire contract has no meaning for
/// a non-positive or fractional version.
fn parse_protocol_version(
    value: Option<&Value>,
) -> Result<McpConfigurationProtocolVersion, McpConfigurationCapabilityIssue> {
    let Some(value) = value else {
        return Err(McpConfigurationCapabilityIssue::ProtocolVersionMissing);
    };
    let Some(number) = value.as_u64() else {
        return Err(McpConfigurationCapabilityIssue::ProtocolVersionNotPositiveInteger);
    };
    let Ok(number) = u32::try_from(number) else {
        return Err(McpConfigurationCapabilityIssue::ProtocolVersionNotPositiveInteger);
    };
    McpConfigurationProtocolVersion::new(number)
}

/// Parses the advertised transport list and rejects duplicates before set construction.
///
/// Duplicate detection happens on the raw token so `["http","http"]` fails even when both tokens
/// would otherwise parse as the same `McpTransportKind`.
fn parse_transports(
    value: Option<&Value>,
) -> Result<McpTransportSet, McpConfigurationCapabilityIssue> {
    let Some(value) = value else {
        return Err(McpConfigurationCapabilityIssue::TransportsMissing);
    };
    let Some(entries) = value.as_array() else {
        return Err(McpConfigurationCapabilityIssue::TransportsNotAnArray);
    };
    let mut kinds = Vec::with_capacity(entries.len());
    let mut seen = BTreeSet::new();
    for entry in entries {
        let Some(token) = entry.as_str() else {
            return Err(McpConfigurationCapabilityIssue::UnknownTransport(
                entry.to_string(),
            ));
        };
        if !seen.insert(token.to_string()) {
            return Err(McpConfigurationCapabilityIssue::DuplicateTransport(
                token.to_string(),
            ));
        }
        kinds.push(McpTransportKind::parse(token)?);
    }
    McpTransportSet::new(kinds)
}

/// Parses the single protocol v1 coordination token; unknown strings disable MCP only.
fn parse_coordination(
    value: Option<&Value>,
) -> Result<McpCoordinationMode, McpConfigurationCapabilityIssue> {
    let Some(value) = value else {
        return Err(McpConfigurationCapabilityIssue::CoordinationMissing);
    };
    let Some(token) = value.as_str() else {
        return Err(McpConfigurationCapabilityIssue::UnknownCoordination(
            value.to_string(),
        ));
    };
    McpCoordinationMode::parse(token)
}

#[cfg(test)]
mod tests {
    use super::{
        McpConfigurationCapability, McpConfigurationCapabilityIssue,
        McpConfigurationProtocolVersion, McpConfigurationRegistration, McpCoordinationMode,
        McpTransportKind, McpTransportSet, parse_mcp_configuration,
    };
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};

    /// Loads one shared registration fixture so Rust and TypeScript assert the same JSON.
    fn fixture(name: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packages/plugin-sdk/tests/fixtures/mcp-configuration/registration")
            .join(name);
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!("read MCP registration fixture {}: {error}", path.display())
        });
        serde_json::from_slice(&bytes).expect("MCP registration fixture is JSON")
    }

    fn parse_fixture(name: &str) -> McpConfigurationRegistration {
        parse_mcp_configuration(Some(&fixture(name)))
    }

    fn http_v1_capability() -> McpConfigurationCapability {
        McpConfigurationCapability::new(
            McpConfigurationProtocolVersion::V1,
            McpTransportSet::new([McpTransportKind::Http]).expect("http set"),
            McpCoordinationMode::WaitForIdleAndRestart,
        )
    }

    /// Older Agent plugins omit the field and remain conversation-capable.
    #[test]
    fn omitted_capability_is_absent() {
        assert_eq!(
            parse_fixture("omitted.json"),
            McpConfigurationRegistration::Absent
        );
    }

    /// Unknown top-level registration fields must not poison a valid capability object.
    #[test]
    fn unknown_top_level_fields_are_ignored() {
        assert_eq!(
            parse_fixture("unknown-top-level-fields.json"),
            McpConfigurationRegistration::Declared(http_v1_capability())
        );
    }

    /// An unsupported protocol version disables MCP without failing the handshake parse.
    #[test]
    fn unknown_protocol_version_invalidates_capability() {
        assert_eq!(
            parse_fixture("unknown-protocol-version.json"),
            McpConfigurationRegistration::Invalid(
                McpConfigurationCapabilityIssue::ProtocolVersionUnsupported { actual: 2 }
            )
        );
    }

    /// Extra capability fields are treated as malformed rather than silently dropped.
    #[test]
    fn malformed_capability_with_unknown_field_is_invalid() {
        assert_eq!(
            parse_fixture("malformed.json"),
            McpConfigurationRegistration::Invalid(McpConfigurationCapabilityIssue::UnknownField(
                "nativeSchema".to_string()
            ))
        );
    }

    /// Duplicate transports are illegal in protocol v1 even when each token is known.
    #[test]
    fn duplicate_transports_invalidate_capability() {
        assert_eq!(
            parse_fixture("duplicate-transports.json"),
            McpConfigurationRegistration::Invalid(
                McpConfigurationCapabilityIssue::DuplicateTransport("http".to_string())
            )
        );
    }

    /// Unknown transport tokens disable MCP without failing handshake.
    #[test]
    fn unknown_transport_invalidates_capability() {
        assert_eq!(
            parse_fixture("unknown-transport.json"),
            McpConfigurationRegistration::Invalid(
                McpConfigurationCapabilityIssue::UnknownTransport("sse".to_string())
            )
        );
    }

    /// An empty transport list is unrepresentable as a protocol v1 capability.
    #[test]
    fn empty_transports_invalidate_capability() {
        assert_eq!(
            parse_fixture("empty-transports.json"),
            McpConfigurationRegistration::Invalid(McpConfigurationCapabilityIssue::TransportsEmpty)
        );
    }

    /// A well-formed HTTP-only v1 declaration is the OpenCode 0.3.0 shape.
    #[test]
    fn valid_http_v1_capability_is_declared() {
        assert_eq!(
            parse_fixture("valid-http-v1.json"),
            McpConfigurationRegistration::Declared(http_v1_capability())
        );
    }

    /// Canonical bytes stay stable so capability revision digests do not churn on key order.
    #[test]
    fn canonical_declaration_bytes_use_sorted_keys_and_transports() {
        let capability = McpConfigurationCapability::new(
            McpConfigurationProtocolVersion::V1,
            McpTransportSet::new([McpTransportKind::Stdio, McpTransportKind::Http])
                .expect("transport set"),
            McpCoordinationMode::WaitForIdleAndRestart,
        );
        assert_eq!(
            capability.canonical_declaration_bytes(),
            br#"{"coordination":"wait_for_idle_and_restart","protocolVersion":1,"transports":["http","stdio"]}"#
        );
    }

    /// Zero is unrepresentable as a protocol version.
    #[test]
    fn zero_protocol_version_is_rejected() {
        assert_eq!(
            parse_mcp_configuration(Some(&json!({
                "mcpConfiguration": {
                    "protocolVersion": 0,
                    "transports": ["http"],
                    "coordination": "wait_for_idle_and_restart"
                }
            }))),
            McpConfigurationRegistration::Invalid(
                McpConfigurationCapabilityIssue::ProtocolVersionNotPositiveInteger
            )
        );
    }

    /// Repairing one malformed declaration into a different malformed declaration must change the digest.
    #[test]
    fn invalid_canonical_bytes_distinguish_malformed_payloads() {
        assert_ne!(
            parse_fixture("malformed.json").canonical_declaration_bytes(),
            parse_fixture("duplicate-transports.json").canonical_declaration_bytes()
        );
    }
}
