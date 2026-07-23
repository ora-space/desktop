use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{Display, Formatter};
use std::marker::PhantomData;
use std::str::FromStr;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use ts_rs::TS;
use uuid::Uuid;

pub const JSON_SAFE_U64_MAX: u64 = 9_007_199_254_740_991;
pub const MAX_OPAQUE_ID_BYTES: usize = 256;
pub const MAX_CONFIGURATION_KEY_BYTES: usize = 512;
pub const MAX_WORKING_DIRECTORY_BYTES: usize = 32 * 1024;
pub const MAX_AGENT_PROMPT_BYTES: usize = 1024 * 1024;

/// Reports why a leaf Agent DTO cannot enter the closed v1 contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentLeafError {
    #[error("{kind} must not be empty")]
    Empty { kind: &'static str },
    #[error("{kind} exceeds its {maximum}-byte limit")]
    TooLong { kind: &'static str, maximum: usize },
    #[error("{kind} has an invalid v1 format")]
    InvalidFormat { kind: &'static str },
    #[error("JSON integer {value} exceeds the JavaScript safe-integer range")]
    JsonIntegerOutOfRange { value: u64 },
    #[error("Agent page limit must be in 1..=100, got {value}")]
    PageLimitOutOfRange { value: u64 },
    #[error("finite JSON number is required")]
    NonFiniteNumber,
}

macro_rules! agent_string_leaf {
    ($name:ident, $kind:literal, $validator:expr) => {
        #[doc = concat!("A validated ", $kind, " encoded as a transparent JSON string.")]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, TS)]
        #[serde(transparent)]
        #[ts(export_to = "agent-contract.ts")]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Validates and constructs a ", $kind, ".")]
            pub fn parse(value: impl Into<String>) -> Result<Self, AgentLeafError> {
                let value = value.into();
                ($validator)(&value)?;
                Ok(Self(value))
            }

            #[doc = concat!("Returns the validated ", $kind, " text.")]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = AgentLeafError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

agent_string_leaf!(
    AgentInstallationId,
    "Agent installation id",
    validate_opaque_id
);
agent_string_leaf!(
    AgentConversationId,
    "Agent conversation id",
    validate_opaque_id
);
agent_string_leaf!(AgentTurnId, "Agent turn id", validate_opaque_id);
agent_string_leaf!(AgentCursor, "Agent cursor", validate_opaque_id);
agent_string_leaf!(AgentResourceId, "Agent resource id", validate_opaque_id);
agent_string_leaf!(AgentToolCallId, "Agent tool-call id", validate_opaque_id);
agent_string_leaf!(ProjectHandle, "project handle", validate_opaque_id);
agent_string_leaf!(WorktreeHandle, "worktree handle", validate_opaque_id);
agent_string_leaf!(
    AgentConfigurationKey,
    "Agent configuration key",
    validate_configuration_key
);
agent_string_leaf!(
    ClientRequestId,
    "client request id",
    validate_client_request_id
);
agent_string_leaf!(
    HostResolvedAbsolutePath,
    "Host-resolved absolute path",
    validate_windows_absolute_path
);
agent_string_leaf!(AgentPrompt, "Agent prompt", validate_prompt);
agent_string_leaf!(Rfc3339Timestamp, "RFC 3339 timestamp", validate_rfc3339);

/// A JavaScript-safe unsigned integer encoded as a JSON number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, TS)]
#[serde(transparent)]
#[ts(export_to = "agent-contract.ts")]
#[ts(type = "number")]
pub struct JsonSafeU64(u64);

impl JsonSafeU64 {
    /// Constructs a count that round-trips through JavaScript number without precision loss.
    pub fn new(value: u64) -> Result<Self, AgentLeafError> {
        if value > JSON_SAFE_U64_MAX {
            return Err(AgentLeafError::JsonIntegerOutOfRange { value });
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }

    /// Returns the next value or fails closed at the protocol maximum.
    pub fn checked_increment(self) -> Result<Self, AgentLeafError> {
        let next = self
            .0
            .checked_add(1)
            .ok_or(AgentLeafError::JsonIntegerOutOfRange { value: u64::MAX })?;
        Self::new(next)
    }
}

impl<'de> Deserialize<'de> for JsonSafeU64 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u64::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// A page size constrained to the Agent v1 maximum of one hundred items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, TS)]
#[serde(transparent)]
#[ts(export_to = "agent-contract.ts")]
pub struct AgentPageLimit(u8);

impl AgentPageLimit {
    /// Validates the inclusive page limit boundary.
    pub fn new(value: u64) -> Result<Self, AgentLeafError> {
        if !(1..=100).contains(&value) {
            return Err(AgentLeafError::PageLimitOutOfRange { value });
        }
        Ok(Self(value as u8))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl<'de> Deserialize<'de> for AgentPageLimit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u64::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// A finite floating-point value that can be represented by JSON.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, TS)]
#[ts(export_to = "agent-contract.ts")]
pub struct FiniteJsonNumber(f64);

impl FiniteJsonNumber {
    /// Rejects NaN and infinities before a configuration value reaches serialization.
    pub fn new(value: f64) -> Result<Self, AgentLeafError> {
        if !value.is_finite() {
            return Err(AgentLeafError::NonFiniteNumber);
        }
        Ok(Self(value))
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl Serialize for FiniteJsonNumber {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for FiniteJsonNumber {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = f64::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Deserializes an optional field while rejecting an explicitly supplied JSON null.
pub(crate) fn deserialize_optional_non_null<'de, D, T>(
    deserializer: D,
) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct NonNullOptionVisitor<T>(PhantomData<T>);

    impl<'de, T> serde::de::Visitor<'de> for NonNullOptionVisitor<T>
    where
        T: Deserialize<'de>,
    {
        type Value = Option<T>;

        fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an omitted field or a non-null value")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Err(E::custom("explicit null is not allowed"))
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Err(E::custom("explicit null is not allowed"))
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            T::deserialize(deserializer).map(Some)
        }
    }

    deserializer.deserialize_option(NonNullOptionVisitor(PhantomData))
}

/// Validates opaque plugin-produced identities without accepting path-like text.
fn validate_opaque_id(value: &str) -> Result<(), AgentLeafError> {
    validate_nonempty_length(value, "opaque Agent identity", MAX_OPAQUE_ID_BYTES)?;
    if value.trim() != value
        || value.chars().any(|character| {
            character == '\0'
                || character == '/'
                || character == '\\'
                || character == ':'
                || character.is_control()
        })
    {
        return Err(invalid("opaque Agent identity"));
    }
    Ok(())
}

/// Applies the dedicated, wider ASCII grammar for configuration keys.
fn validate_configuration_key(value: &str) -> Result<(), AgentLeafError> {
    validate_nonempty_length(
        value,
        "Agent configuration key",
        MAX_CONFIGURATION_KEY_BYTES,
    )?;
    if !value.is_ascii()
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || value
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
    {
        return Err(invalid("Agent configuration key"));
    }
    Ok(())
}

/// Requires the Host-issued UUID to remain canonical lower-case hyphenated text.
fn validate_client_request_id(value: &str) -> Result<(), AgentLeafError> {
    validate_nonempty_length(value, "client request id", 36)?;
    let parsed = Uuid::parse_str(value).map_err(|_| invalid("client request id"))?;
    if parsed.hyphenated().to_string() != value {
        return Err(invalid("client request id"));
    }
    Ok(())
}

/// Accepts canonical absolute Windows drive, UNC, and extended-length paths from the Host.
fn validate_windows_absolute_path(value: &str) -> Result<(), AgentLeafError> {
    validate_nonempty_length(
        value,
        "Host-resolved absolute path",
        MAX_WORKING_DIRECTORY_BYTES,
    )?;
    if value.contains('\0') || !is_windows_absolute(value) {
        return Err(invalid("Host-resolved absolute path"));
    }
    Ok(())
}

/// Preserves prompt whitespace while applying the v1 byte and NUL bounds.
fn validate_prompt(value: &str) -> Result<(), AgentLeafError> {
    validate_nonempty_length(value, "Agent prompt", MAX_AGENT_PROMPT_BYTES)?;
    if value.contains('\0') {
        return Err(invalid("Agent prompt"));
    }
    Ok(())
}

/// Uses the RFC 3339 parser and retains the authored offset form.
fn validate_rfc3339(value: &str) -> Result<(), AgentLeafError> {
    validate_nonempty_length(value, "RFC 3339 timestamp", 64)?;
    if !value.is_ascii() || OffsetDateTime::parse(value, &Rfc3339).is_err() {
        return Err(invalid("RFC 3339 timestamp"));
    }
    Ok(())
}

/// Recognizes Windows absolute path prefixes without consulting ambient filesystem state.
fn is_windows_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    (bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/'))
        || value.starts_with("\\\\")
        || value.starts_with("//")
}

/// Applies a non-empty UTF-8 byte limit shared by Agent leaf types.
fn validate_nonempty_length(
    value: &str,
    kind: &'static str,
    maximum: usize,
) -> Result<(), AgentLeafError> {
    if value.is_empty() {
        return Err(AgentLeafError::Empty { kind });
    }
    if value.len() > maximum {
        return Err(AgentLeafError::TooLong { kind, maximum });
    }
    Ok(())
}

/// Builds a stable leaf-format failure without attacker-controlled detail.
fn invalid(kind: &'static str) -> AgentLeafError {
    AgentLeafError::InvalidFormat { kind }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentConfigurationKey, AgentInstallationId, AgentPageLimit, AgentPrompt, ClientRequestId,
        FiniteJsonNumber, HostResolvedAbsolutePath, JSON_SAFE_U64_MAX, JsonSafeU64,
        Rfc3339Timestamp,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies leaf newtypes serialize as primitives rather than `{ value }` wrappers.
    #[test]
    fn serializes_leaf_types_as_json_primitives() {
        let installation = AgentInstallationId::parse("install-1")
            .unwrap_or_else(|error| panic!("expected installation id: {error}"));
        let prompt = AgentPrompt::parse(" keep whitespace \n")
            .unwrap_or_else(|error| panic!("expected prompt: {error}"));
        let installation_json = serde_json::to_value(installation)
            .unwrap_or_else(|error| panic!("expected installation serialization: {error}"));
        let prompt_json = serde_json::to_value(prompt)
            .unwrap_or_else(|error| panic!("expected prompt serialization: {error}"));
        let count =
            JsonSafeU64::new(1).unwrap_or_else(|error| panic!("expected safe count: {error}"));
        let count_json = serde_json::to_value(count)
            .unwrap_or_else(|error| panic!("expected count serialization: {error}"));
        assert_eq!(installation_json, json!("install-1"));
        assert_eq!(prompt_json, json!(" keep whitespace \n"));
        assert_eq!(count_json, json!(1));
    }

    /// Covers the explicit numeric, UUID, path, key, and timestamp boundaries.
    #[test]
    fn validates_agent_leaf_boundaries() {
        assert!(JsonSafeU64::new(JSON_SAFE_U64_MAX).is_ok());
        assert!(JsonSafeU64::new(JSON_SAFE_U64_MAX + 1).is_err());
        assert!(AgentPageLimit::new(1).is_ok());
        assert!(AgentPageLimit::new(100).is_ok());
        assert!(AgentPageLimit::new(101).is_err());
        assert!(FiniteJsonNumber::new(f64::INFINITY).is_err());
        assert_eq!(
            AgentConfigurationKey::parse("agent.model-name").is_ok(),
            true
        );
        assert!(AgentConfigurationKey::parse("agent/model").is_err());
        assert_eq!(
            ClientRequestId::parse("550e8400-e29b-41d4-a716-446655440000").is_ok(),
            true
        );
        assert_eq!(
            HostResolvedAbsolutePath::parse("D:\\workspace").is_ok(),
            true
        );
        assert_eq!(
            HostResolvedAbsolutePath::parse("relative\\workspace").is_err(),
            true
        );
        assert_eq!(
            Rfc3339Timestamp::parse("2026-07-16T12:00:00Z").is_ok(),
            true
        );
    }
}
