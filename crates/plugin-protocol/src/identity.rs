use serde::{Deserialize, Deserializer, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use ts_rs::TS;
use uuid::Uuid;

const SHA256_HEX_LENGTH: usize = 64;

/// Reports why a protocol identity cannot be represented by its v1 canonical form.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    #[error("{kind} must not be empty")]
    Empty { kind: &'static str },
    #[error("{kind} exceeds its {maximum}-byte limit")]
    TooLong { kind: &'static str, maximum: usize },
    #[error("{kind} has an invalid v1 format")]
    InvalidFormat { kind: &'static str },
}

macro_rules! validated_string {
    ($name:ident, $kind:literal, $validator:expr, $export:literal) => {
        #[doc = concat!("A validated ", $kind, " serialized as a transparent JSON string.")]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, TS)]
        #[serde(transparent)]
        #[ts(export_to = $export)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Validates and constructs a canonical ", $kind, ".")]
            pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
                let value = value.into();
                ($validator)(&value)?;
                Ok(Self(value))
            }

            #[doc = concat!("Returns the canonical ", $kind, " text.")]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[doc = concat!("Consumes the ", $kind, " and returns its canonical text.")]
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdentityError;

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

validated_string!(PluginId, "plugin id", validate_plugin_id, "plugin-types.ts");
validated_string!(
    AgentProviderId,
    "Agent provider id",
    validate_agent_provider_id,
    "agent-contract.ts"
);
validated_string!(
    PluginVersion,
    "plugin version",
    validate_semver,
    "plugin-types.ts"
);
validated_string!(
    PluginRelativePath,
    "plugin relative path",
    validate_plugin_relative_path,
    "plugin-types.ts"
);
validated_string!(
    ContentDigest,
    "content digest",
    validate_content_digest,
    "plugin-types.ts"
);
validated_string!(
    ContentOwnerId,
    "content owner id",
    validate_content_owner,
    "plugin-types.ts"
);
validated_string!(
    OperationId,
    "operation id",
    validate_uuid,
    "plugin-types.ts"
);
validated_string!(
    CandidateAuditId,
    "candidate audit id",
    validate_uuid,
    "plugin-types.ts"
);

/// A global Agent provider identity that never relies on delimiter concatenation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agent-contract.ts")]
pub struct AgentProviderKey {
    pub plugin_id: PluginId,
    pub provider_id: AgentProviderId,
}

/// Validates the canonical dotted plugin identity grammar and Windows-safe labels.
fn validate_plugin_id(value: &str) -> Result<(), IdentityError> {
    validate_length(value, "plugin id", 128)?;
    if !value.is_ascii() || value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(invalid("plugin id"));
    }

    let labels = value.split('.').collect::<Vec<_>>();
    if labels.len() < 2
        || labels.iter().any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || label.bytes().any(|byte| {
                    !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                })
                || is_windows_device_name(label)
        })
    {
        return Err(invalid("plugin id"));
    }
    Ok(())
}

/// Validates a plugin-local provider id without applying the dotted plugin-id grammar.
fn validate_agent_provider_id(value: &str) -> Result<(), IdentityError> {
    validate_length(value, "Agent provider id", 63)?;
    if !value.is_ascii()
        || value.starts_with('-')
        || value.ends_with('-')
        || value
            .bytes()
            .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    {
        return Err(invalid("Agent provider id"));
    }
    Ok(())
}

/// Uses the SemVer parser and requires its canonical rendering to match the input.
fn validate_semver(value: &str) -> Result<(), IdentityError> {
    validate_length(value, "plugin version", 256)?;
    let parsed = semver::Version::parse(value).map_err(|_| invalid("plugin version"))?;
    if parsed.to_string() != value {
        return Err(invalid("plugin version"));
    }
    Ok(())
}

/// Validates a portable plugin-relative path before any filesystem join occurs.
fn validate_plugin_relative_path(value: &str) -> Result<(), IdentityError> {
    validate_length(value, "plugin relative path", 512)?;
    if value.contains('\0') {
        return Err(invalid("plugin relative path"));
    }
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.contains(':')
        || value.ends_with('/')
        || value.ends_with('\\')
    {
        return Err(invalid("plugin relative path"));
    }

    let segments = value.split(['/', '\\']);
    for segment in segments {
        if segment.is_empty()
            || matches!(segment, "." | "..")
            || segment.ends_with('.')
            || segment.ends_with(' ')
            || is_windows_device_name(segment)
        {
            return Err(invalid("plugin relative path"));
        }
    }
    Ok(())
}

/// Validates the display digest form retained in receipts and state.
fn validate_content_digest(value: &str) -> Result<(), IdentityError> {
    validate_sha256_prefixed(value, "sha256:", "content digest")
}

/// Validates the path-safe content owner form used beneath plugin-data.
fn validate_content_owner(value: &str) -> Result<(), IdentityError> {
    validate_sha256_prefixed(value, "sha256-", "content owner id")
}

/// Validates canonical lowercase UUID text used for operation and audit identities.
fn validate_uuid(value: &str) -> Result<(), IdentityError> {
    validate_length(value, "UUID identity", 36)?;
    let parsed = Uuid::parse_str(value).map_err(|_| invalid("UUID identity"))?;
    if parsed.hyphenated().to_string() != value {
        return Err(invalid("UUID identity"));
    }
    Ok(())
}

/// Validates a lower-hex SHA-256 string with an exact protocol prefix.
fn validate_sha256_prefixed(
    value: &str,
    prefix: &str,
    kind: &'static str,
) -> Result<(), IdentityError> {
    let Some(hex) = value.strip_prefix(prefix) else {
        return Err(invalid(kind));
    };
    if hex.len() != SHA256_HEX_LENGTH
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(kind));
    }
    Ok(())
}

/// Applies a non-empty UTF-8 byte limit shared by all string identities.
fn validate_length(value: &str, kind: &'static str, maximum: usize) -> Result<(), IdentityError> {
    if value.is_empty() {
        return Err(IdentityError::Empty { kind });
    }
    if value.len() > maximum {
        return Err(IdentityError::TooLong { kind, maximum });
    }
    Ok(())
}

/// Recognizes Windows reserved DOS device names, including extension forms.
fn is_windows_device_name(value: &str) -> bool {
    let base = value
        .split('.')
        .next()
        .unwrap_or(value)
        .to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'))
}

/// Builds a stable invalid-format error without exposing parser-specific details.
fn invalid(kind: &'static str) -> IdentityError {
    IdentityError::InvalidFormat { kind }
}

#[cfg(test)]
mod tests {
    use super::{AgentProviderId, ContentDigest, ContentOwnerId, PluginId, PluginRelativePath};
    use pretty_assertions::assert_eq;

    /// Covers canonical and rejected plugin/provider identities from the v1 grammar.
    #[test]
    fn validates_plugin_and_provider_identities() {
        assert!(PluginId::parse("ora.claude-code").is_ok());
        assert!(PluginId::parse("Ora.claude-code").is_err());
        assert!(PluginId::parse("ora.con").is_err());
        assert!(AgentProviderId::parse("claude-code").is_ok());
        assert!(AgentProviderId::parse("-claude").is_err());
        assert!(AgentProviderId::parse("claude.code").is_err());
        assert!(AgentProviderId::parse("中文").is_err());
    }

    /// Rejects path traversal, alternate streams, devices, and ambiguous path segments.
    #[test]
    fn validates_plugin_relative_paths() {
        assert!(PluginRelativePath::parse("dist/index.js").is_ok());
        assert!(PluginRelativePath::parse("../index.js").is_err());
        assert!(PluginRelativePath::parse("C:\\index.js").is_err());
        assert!(PluginRelativePath::parse("dist/con.js").is_err());
        assert_eq!(
            PluginRelativePath::parse("dist/file.js:secret").is_err(),
            true
        );
    }

    /// Keeps receipt and path-safe owner digest encodings deliberately distinct.
    #[test]
    fn validates_digest_encodings() {
        let hex = "a".repeat(64);
        assert!(ContentDigest::parse(format!("sha256:{hex}")).is_ok());
        assert!(ContentOwnerId::parse(format!("sha256-{hex}")).is_ok());
        assert_eq!(
            ContentOwnerId::parse(format!("sha256:{hex}")).is_err(),
            true
        );
    }
}
