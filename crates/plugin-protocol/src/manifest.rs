use crate::{
    AgentProviderId, PluginId, PluginRelativePath, PluginVersion, StrictJsonError,
    parse_strict_json,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use ts_rs::TS;

pub const MANIFEST_VERSION_V1: u32 = 1;
pub const PLUGIN_API_VERSION_V1: u32 = 1;
pub const AGENT_CONTRACT_VERSION_V1: u32 = 1;
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;
pub const MAX_MANIFEST_JSON_DEPTH: usize = 64;
pub const MAX_CONTRIBUTIONS: usize = 64;

/// A parsed package.json containing standard package metadata and strict Ora metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin-types.ts")]
pub struct PluginPackageManifest {
    pub name: String,
    pub version: PluginVersion,
    #[serde(rename = "type", default)]
    #[ts(optional)]
    pub module_type: Option<PackageModuleType>,
    pub ora: PluginManifest,
}

/// The only package module mode supported by the materialized Agent v1 format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin-types.ts")]
pub enum PackageModuleType {
    Module,
}

/// The closed v1 plugin-kind union; fields illegal for a kind cannot be represented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[ts(export_to = "plugin-types.ts")]
pub enum PluginManifest {
    Agent {
        manifest_version: u32,
        id: PluginId,
        display_name: String,
        main: PluginRelativePath,
        engines: AgentEngines,
        contributes: AgentContributions,
    },
    Workbench {
        manifest_version: u32,
        id: PluginId,
        display_name: String,
        engines: WorkbenchEngines,
        contributes: WorkbenchContributions,
    },
}

impl PluginManifest {
    /// Returns the canonical identity shared by directories, state, catalog, and registry.
    pub fn id(&self) -> &PluginId {
        match self {
            Self::Agent { id, .. } | Self::Workbench { id, .. } => id,
        }
    }

    /// Returns the user-facing name after manifest budget validation.
    pub fn display_name(&self) -> &str {
        match self {
            Self::Agent { display_name, .. } | Self::Workbench { display_name, .. } => display_name,
        }
    }

    /// Returns the closed plugin kind without exposing kind-specific optional fields.
    pub fn kind(&self) -> PluginKind {
        match self {
            Self::Agent { .. } => PluginKind::Agent,
            Self::Workbench { .. } => PluginKind::Workbench,
        }
    }

    /// Returns Agent contributions only for the executable Agent variant.
    pub fn agent_contributions(&self) -> Option<&AgentContributions> {
        match self {
            Self::Agent { contributes, .. } => Some(contributes),
            Self::Workbench { .. } => None,
        }
    }
}

/// The manifest kind known to management and support-policy decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin-types.ts")]
pub enum PluginKind {
    Agent,
    Workbench,
}

/// Compatibility requirements for an executable Agent plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "plugin-types.ts")]
pub struct AgentEngines {
    pub ora: EngineRange,
    pub plugin_api: u32,
    pub bun: EngineRange,
}

/// Compatibility requirements for a catalog-only Workbench v1 plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "plugin-types.ts")]
pub struct WorkbenchEngines {
    pub ora: EngineRange,
}

/// A validated SemVer requirement string retained exactly as authored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(transparent)]
#[ts(export_to = "plugin-types.ts")]
pub struct EngineRange(String);

impl EngineRange {
    /// Parses the whitespace-separated comparator form used by Ora manifests.
    pub fn parse(value: impl Into<String>) -> Result<Self, ManifestParseError> {
        let value = value.into();
        if value.is_empty() || value.len() > 256 || !value.is_ascii() {
            return Err(ManifestParseError::InvalidEngineRange);
        }
        parse_version_requirement(&value)?;
        Ok(Self(value))
    }

    /// Returns the exact authored range for diagnostics and generated types.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Tests a canonical SemVer against this range without defining a compatibility shim.
    pub fn matches(&self, version: &PluginVersion) -> bool {
        let requirement = parse_version_requirement(&self.0);
        let version = semver::Version::parse(version.as_str());
        matches!((requirement, version), (Ok(requirement), Ok(version)) if requirement.matches(&version))
    }
}

impl Display for EngineRange {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for EngineRange {
    type Err = ManifestParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for EngineRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// Agent contributions statically declared by the immutable manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "plugin-types.ts")]
pub struct AgentContributions {
    pub agents: Vec<AgentContribution>,
}

/// One plugin-local provider registration allowed by Agent Contract v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "plugin-types.ts")]
pub struct AgentContribution {
    pub id: AgentProviderId,
    pub display_name: String,
    pub contract_version: u32,
}

/// Workbench v1 deliberately exposes only a non-executable placeholder contribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "plugin-types.ts")]
pub struct WorkbenchContributions {
    pub workbench: WorkbenchContribution,
}

/// The exact non-executable Workbench v1 schema marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "plugin-types.ts")]
pub struct WorkbenchContribution {
    pub schema_version: u32,
}

/// Classifies manifest parsing and v1 schema failures before compatibility checks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestParseError {
    #[error("package.json exceeds its {maximum}-byte limit")]
    TooLarge { maximum: usize },
    #[error(transparent)]
    Json(#[from] StrictJsonError),
    #[error("manifest envelope is missing a non-negative integer ora.manifestVersion")]
    MissingManifestVersion,
    #[error("manifest schema version {manifest_version} is not supported")]
    UnsupportedManifestVersion { manifest_version: u64 },
    #[error("known manifest v1 schema is invalid: {message}")]
    InvalidSchema { message: String },
    #[error("manifest engine range is invalid")]
    InvalidEngineRange,
    #[error("manifest violates v1 invariant: {message}")]
    InvalidInvariant { message: String },
}

/// Parses package.json with duplicate-key/depth protection and validates v1 invariants.
pub fn parse_plugin_manifest(bytes: &[u8]) -> Result<PluginPackageManifest, ManifestParseError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestParseError::TooLarge {
            maximum: MAX_MANIFEST_BYTES,
        });
    }
    let value = parse_strict_json(bytes, MAX_MANIFEST_JSON_DEPTH)?;
    let manifest_version = value
        .get("ora")
        .and_then(|ora| ora.get("manifestVersion"))
        .and_then(serde_json::Value::as_u64)
        .ok_or(ManifestParseError::MissingManifestVersion)?;
    if manifest_version != u64::from(MANIFEST_VERSION_V1) {
        return Err(ManifestParseError::UnsupportedManifestVersion { manifest_version });
    }
    let manifest = serde_json::from_value::<PluginPackageManifest>(value).map_err(|error| {
        ManifestParseError::InvalidSchema {
            message: error.to_string(),
        }
    })?;
    validate_manifest_invariants(&manifest)?;
    Ok(manifest)
}

/// Enforces semantic constraints that cannot be represented by serde object shapes alone.
fn validate_manifest_invariants(package: &PluginPackageManifest) -> Result<(), ManifestParseError> {
    if package.name.is_empty() || package.name.len() > 512 {
        return Err(invariant("package name must contain 1..=512 UTF-8 bytes"));
    }
    if package.ora.display_name().chars().count() > 128 || package.ora.display_name().is_empty() {
        return Err(invariant(
            "displayName must contain 1..=128 Unicode scalar values",
        ));
    }

    match &package.ora {
        PluginManifest::Agent {
            manifest_version,
            engines,
            contributes,
            ..
        } => {
            require_v1_manifest_version(*manifest_version)?;
            if package.module_type != Some(PackageModuleType::Module) {
                return Err(invariant("Agent v1 requires top-level type=module"));
            }
            if engines.plugin_api != PLUGIN_API_VERSION_V1 {
                return Err(invariant("Agent v1 requires engines.pluginApi=1"));
            }
            if contributes.agents.is_empty() || contributes.agents.len() > MAX_CONTRIBUTIONS {
                return Err(invariant("Agent contribution count must be in 1..=64"));
            }
            let mut ids = BTreeSet::new();
            for contribution in &contributes.agents {
                if contribution.contract_version != AGENT_CONTRACT_VERSION_V1 {
                    return Err(invariant("Agent contribution contractVersion must equal 1"));
                }
                if contribution.display_name.is_empty()
                    || contribution.display_name.chars().count() > 128
                {
                    return Err(invariant(
                        "Agent displayName must contain 1..=128 Unicode scalar values",
                    ));
                }
                if !ids.insert(contribution.id.clone()) {
                    return Err(invariant("Agent contribution ids must be unique"));
                }
            }
        }
        PluginManifest::Workbench {
            manifest_version,
            contributes,
            ..
        } => {
            require_v1_manifest_version(*manifest_version)?;
            if contributes.workbench.schema_version != 1 {
                return Err(invariant("Workbench v1 schemaVersion must equal 1"));
            }
        }
    }
    Ok(())
}

/// Keeps schema-version equality explicit even after envelope routing.
fn require_v1_manifest_version(manifest_version: u32) -> Result<(), ManifestParseError> {
    if manifest_version != MANIFEST_VERSION_V1 {
        return Err(invariant("manifestVersion must equal 1"));
    }
    Ok(())
}

/// Parses Ora's whitespace comparator spelling through semver's comma-separated grammar.
fn parse_version_requirement(value: &str) -> Result<semver::VersionReq, ManifestParseError> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(", ");
    semver::VersionReq::parse(&normalized).map_err(|_| ManifestParseError::InvalidEngineRange)
}

/// Builds a stable invariant error for catalog diagnostics.
fn invariant(message: impl Into<String>) -> ManifestParseError {
    ManifestParseError::InvalidInvariant {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ManifestParseError, PluginKind, parse_plugin_manifest};
    use pretty_assertions::assert_eq;

    const AGENT_MANIFEST: &str = r#"{
        "name":"@ora-plugins/claude-code",
        "version":"0.1.0",
        "type":"module",
        "ora":{
            "manifestVersion":1,
            "id":"ora.claude-code",
            "displayName":"Claude Code",
            "kind":"agent",
            "main":"dist/index.js",
            "engines":{"ora":">=0.1.0 <0.2.0","pluginApi":1,"bun":">=1.0.0 <2.0.0"},
            "contributes":{"agents":[{"id":"claude-code","displayName":"Claude Code","contractVersion":1}]}
        }
    }"#;

    /// Parses the exact Agent v1 shape and preserves kind-specific data in the enum.
    #[test]
    fn parses_agent_manifest() {
        let manifest = parse_plugin_manifest(AGENT_MANIFEST.as_bytes())
            .unwrap_or_else(|error| panic!("expected Agent manifest to parse: {error}"));
        assert_eq!(manifest.ora.kind(), PluginKind::Agent);
        assert_eq!(manifest.ora.id().as_str(), "ora.claude-code");
    }

    /// Routes unknown schema versions separately from known-v1 strict-field errors.
    #[test]
    fn distinguishes_unknown_version_from_invalid_v1() {
        let unknown = AGENT_MANIFEST.replace("\"manifestVersion\":1", "\"manifestVersion\":2");
        assert_eq!(
            parse_plugin_manifest(unknown.as_bytes()),
            Err(ManifestParseError::UnsupportedManifestVersion {
                manifest_version: 2,
            })
        );

        let invalid = AGENT_MANIFEST.replace(
            "\"displayName\":\"Claude Code\",\n            \"kind\"",
            "\"displayName\":\"Claude Code\",\n            \"future\":true,\n            \"kind\"",
        );
        assert_eq!(
            matches!(
                parse_plugin_manifest(invalid.as_bytes()),
                Err(ManifestParseError::InvalidSchema { .. })
            ),
            true
        );
    }

    /// Rejects duplicate providers rather than allowing activation to select one implicitly.
    #[test]
    fn rejects_duplicate_agent_contributions() {
        let duplicate = AGENT_MANIFEST.replace(
            "}]}",
            "},{\"id\":\"claude-code\",\"displayName\":\"Duplicate\",\"contractVersion\":1}]}",
        );
        assert_eq!(
            matches!(
                parse_plugin_manifest(duplicate.as_bytes()),
                Err(ManifestParseError::InvalidInvariant { .. })
            ),
            true
        );
    }
}
