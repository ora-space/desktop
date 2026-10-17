//! Plugin install and removal executions of a Workspace operation's plugin step.
//!
//! Cloud assembles every payload from its catalog snapshot; the Node never resolves a marketplace
//! and trusts the SHA-256, not the download source.

use crate::{MessageValidationError, NodeId, NodeRuntimeIdentity};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Canonical Plugin ID, `<namespace>/<identifier>`; version is never part of it.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PluginId(String);

impl PluginId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the canonical wire representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Accepts exactly one separator with non-empty, whitespace-free segments on both sides; the
    /// segment grammar itself belongs to the plugin manifest rules, not to this transport.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        let valid = self
            .0
            .split_once('/')
            .is_some_and(|(namespace, identifier)| {
                !namespace.is_empty()
                    && !identifier.is_empty()
                    && !identifier.contains('/')
                    && !self.0.chars().any(|c| c.is_whitespace() || c.is_control())
            });
        if valid {
            return Ok(());
        }
        Err(MessageValidationError::InvalidPluginId)
    }
}

/// Exact plugin package version selected by Cloud.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PluginVersion(String);

impl PluginVersion {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the version exactly as carried on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Lowercase hexadecimal SHA-256 digest.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the digest exactly as carried on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Requires the canonical lowercase form so equal digests always compare equal as strings.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        if self.0.len() == 64
            && self
                .0
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Ok(());
        }
        Err(MessageValidationError::InvalidSha256)
    }
}

/// One package download; the digest is checked before anything reaches the plugin root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDownload {
    pub url: String,
    pub sha256: Sha256Digest,
}

/// A release built for one Rust target triple.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginTargetDownload {
    pub target: String,
    pub download: PluginDownload,
}

/// Either one universal package or per-target packages; the Node picks the one for its host.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginRelease {
    Universal { download: PluginDownload },
    Targets { targets: Vec<PluginTargetDownload> },
}

/// The Agent declaration identity one install plan freezes from Cloud's catalog snapshot.
///
/// The association is the release's publication binding minus its marketplace-only copy path:
/// what a Node must prove is that the package's in-archive declaration bytes carry exactly this
/// digest, not where the sync read a copy of them. The plugin id, version, target, and package
/// digest all live on the surrounding plan, which is what makes the association a binding to an
/// exact release rather than a free-floating schema claim.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginAgentConfiguration {
    /// Declaration schema version the associated copy declares.
    pub schema_version: u32,
    /// Lowercase hexadecimal SHA-256 of the exact declaration bytes.
    pub declaration_sha256: Sha256Digest,
}

/// One plugin to install at an exact version.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginInstall {
    pub plugin_id: PluginId,
    pub version: PluginVersion,
    pub release: PluginRelease,
    /// Present when the catalog entry the plan froze declared an Agent configuration
    /// association; absent plans install without a declaration binding to prove.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_configuration: Option<PluginAgentConfiguration>,
}

/// One plugin to remove; removing an absent plugin succeeds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRemoval {
    pub plugin_id: PluginId,
    pub version: PluginVersion,
}

/// Installs a non-empty plugin set into the Workspace's plugin root without starting any plugin.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstallPluginsSpec {
    pub node_id: NodeId,
    pub plugins: Vec<PluginInstall>,
}

/// Removes a non-empty plugin set from the Workspace's plugin root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemovePluginsSpec {
    pub node_id: NodeId,
    pub plugins: Vec<PluginRemoval>,
}

impl InstallPluginsSpec {
    /// Rejects payloads a Node could not verify before download.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        validate_plugin_set(
            &self.node_id,
            self.plugins.iter().map(|p| (&p.plugin_id, &p.version)),
        )?;
        for plugin in &self.plugins {
            match &plugin.release {
                PluginRelease::Universal { download } => validate_download(download)?,
                PluginRelease::Targets { targets } => {
                    if targets.is_empty() {
                        return Err(MessageValidationError::InvalidPluginRelease);
                    }
                    let mut seen = HashSet::with_capacity(targets.len());
                    for target in targets {
                        // One package per target keeps host selection deterministic.
                        if target.target.trim().is_empty() || !seen.insert(target.target.as_str()) {
                            return Err(MessageValidationError::InvalidPluginRelease);
                        }
                        validate_download(&target.download)?;
                    }
                }
            }
            if let Some(agent) = &plugin.agent_configuration {
                // A zero version cannot name a declaration schema; support beyond the structural
                // check belongs to the installer that compiles the declaration bytes.
                if agent.schema_version == 0 {
                    return Err(MessageValidationError::EmptyField {
                        field: "agent_configuration.schema_version",
                    });
                }
                agent.declaration_sha256.validate()?;
            }
        }
        Ok(())
    }
}

impl RemovePluginsSpec {
    /// Rejects empty or ambiguous removal sets.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        validate_plugin_set(
            &self.node_id,
            self.plugins.iter().map(|p| (&p.plugin_id, &p.version)),
        )
    }
}

/// Requires a target Node and a non-empty set naming each plugin once; an empty set never reaches
/// a Node because Cloud advances the step without an execution.
fn validate_plugin_set<'a>(
    node_id: &NodeId,
    plugins: impl ExactSizeIterator<Item = (&'a PluginId, &'a PluginVersion)>,
) -> Result<(), MessageValidationError> {
    if node_id.is_empty() {
        return Err(MessageValidationError::EmptyField { field: "node_id" });
    }
    if plugins.len() == 0 {
        return Err(MessageValidationError::EmptyPluginSet);
    }
    let mut seen = HashSet::with_capacity(plugins.len());
    for (plugin_id, version) in plugins {
        plugin_id.validate()?;
        if version.0.trim().is_empty() {
            return Err(MessageValidationError::EmptyField { field: "version" });
        }
        if !seen.insert(plugin_id) {
            return Err(MessageValidationError::DuplicatePlugin);
        }
    }
    Ok(())
}

/// Accepts only HTTP(S) download locations with a canonical digest.
fn validate_download(download: &PluginDownload) -> Result<(), MessageValidationError> {
    let scheme_ok = url::Url::parse(&download.url)
        .is_ok_and(|url| matches!(url.scheme(), "https" | "http") && url.host_str().is_some());
    if !scheme_ok {
        return Err(MessageValidationError::InvalidPluginRelease);
    }
    download.sha256.validate()
}

/// Bounded per-plugin failure codes; raw diagnostics stay in Node logs.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginFailureCode {
    DownloadFailed,
    ChecksumMismatch,
    NoMatchingTarget,
    InvalidPackage,
    InstallFailed,
    /// A running Agent session uses the plugin, so it was neither replaced nor removed.
    PluginInUse,
}

/// Outcome of one planned plugin.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginItemOutcome {
    Installed { version: PluginVersion },
    Removed {},
    Failed { failure: PluginFailureCode },
}

/// Result of one planned plugin; a failed item does not fail the execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginItemResult {
    pub plugin_id: PluginId,
    pub outcome: PluginItemOutcome,
}

/// Every planned plugin has a result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginsCompleted {
    pub node: NodeRuntimeIdentity,
    pub items: Vec<PluginItemResult>,
}

/// Why the execution as a whole could not run.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginsFailureCode {
    /// The Workspace's plugin root or the Node ledger could not be used.
    PluginRootUnavailable,
    /// Stopped before every item had a result; installs are idempotent, so a new execution may
    /// simply retry.
    Interrupted,
}

/// No item result is reported when the execution as a whole failed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginsFailed {
    pub node: NodeRuntimeIdentity,
    pub failure: PluginsFailureCode,
}

/// Plugin execution's disjoint wire tags cannot be decoded as another business's result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "result", rename_all = "snake_case")]
pub enum PluginExecutionResult {
    PluginsCompleted(PluginsCompleted),
    PluginsFailed(PluginsFailed),
}

impl PluginExecutionResult {
    /// Checks origin and that each plugin is reported at most once; matching items against the
    /// planned set needs the input and belongs to the receiver.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        self.node()
            .validate()
            .map_err(|field| MessageValidationError::EmptyField { field })?;
        let Self::PluginsCompleted(completed) = self else {
            return Ok(());
        };
        let mut seen = HashSet::with_capacity(completed.items.len());
        for item in &completed.items {
            item.plugin_id.validate()?;
            if !seen.insert(&item.plugin_id) {
                return Err(MessageValidationError::DuplicatePlugin);
            }
        }
        Ok(())
    }

    /// Preserves the original incarnation when a restarted Node reports stored evidence.
    pub(crate) fn node(&self) -> &NodeRuntimeIdentity {
        match self {
            Self::PluginsCompleted(result) => &result.node,
            Self::PluginsFailed(result) => &result.node,
        }
    }
}
