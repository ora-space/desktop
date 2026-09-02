use crate::{
    CapabilityRequirement, ConsumerAdapterIdentity, ConsumerIdentity, ConsumerRevisionId, Digest,
    EffectKind, EffectResourceId, EffectScopeId, EffectTargetId, ResourceAdapterIdentity,
    ResourceKey,
};
use ora_utils::path::PortableRelativePath;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display, Formatter};
use std::path::PathBuf;
use thiserror::Error;

/// Lifecycle of the isolation root that owns every Effect convergence obligation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectScopeLifecycle {
    Active,
    Retiring,
}

/// Isolation root for Desired State, Targets, Resources, and ownership.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectScope {
    pub identity: EffectScopeId,
    pub lifecycle: EffectScopeLifecycle,
}

/// Complete immutable capabilities of one Consumer Revision.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilitySet {
    pub effect_protocols: BTreeMap<EffectKind, u32>,
    pub materialization_contracts: BTreeSet<String>,
    pub coordination_contracts: BTreeSet<String>,
    pub readiness_contracts: BTreeSet<String>,
}

/// Immutable declaration and capability snapshot for one stable Consumer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConsumerRevision {
    pub identity: ConsumerRevisionId,
    pub consumer: ConsumerIdentity,
    pub capabilities: CapabilitySet,
    pub declaration_digest: Digest,
}

/// Lifecycle distinguishes permanent retirement from a temporary runtime disconnect.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerLifecycle {
    Declared,
    Retiring,
}

/// Stable external runtime identity plus the exact capability revision currently declared.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Consumer {
    pub identity: ConsumerIdentity,
    pub adapter: ConsumerAdapterIdentity,
    pub current_revision: ConsumerRevisionId,
    pub lifecycle: ConsumerLifecycle,
}

/// Lifecycle of a Consumer's convergence instance inside one Scope.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetLifecycle {
    Active,
    Retiring,
}

/// Generic scheduling and readiness boundary for one Consumer in one Scope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectTarget {
    pub identity: EffectTargetId,
    pub scope: EffectScopeId,
    pub consumer: ConsumerIdentity,
    pub consumer_revision: ConsumerRevisionId,
    pub lifecycle: TargetLifecycle,
}

/// A normalized, safe Workspace-relative filesystem Resource path.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ResourcePath(PortableRelativePath);

impl ResourcePath {
    /// Parses a Consumer declaration and refuses the Workspace root itself.
    pub fn parse(value: &str) -> Result<Self, DeclarationError> {
        let path = PortableRelativePath::parse(value)
            .map_err(|_| DeclarationError::UnsafeRelativePath(value.to_string()))?;
        if path.is_root() {
            return Err(DeclarationError::WorkspaceRootResource);
        }
        Ok(Self(path))
    }

    /// Returns the normalized slash-separated persistence representation.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Reconstructs the path with host-native components for a filesystem adapter.
    pub fn to_path_buf(&self) -> PathBuf {
        self.0.to_path_buf()
    }
}

impl Display for ResourcePath {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for ResourcePath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ResourcePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

/// Stable name of an adapter-compatible materialization representation.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct MaterializationFormat(String);

impl MaterializationFormat {
    /// Returns the first-version directory tree format used for Skill packages.
    pub fn skill_directory_v1() -> Self {
        Self("ora/skill-directory.v1".to_string())
    }

    /// Refuses an empty format because unknown formats must never be guessed by an adapter.
    pub fn parse(value: impl Into<String>) -> Result<Self, DeclarationError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(DeclarationError::EmptyMaterializationFormat);
        }
        Ok(Self(value))
    }

    /// Returns the versioned adapter format identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for MaterializationFormat {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A validated filesystem directory locator interpreted only by its Resource adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FilesystemDirectoryDescriptor {
    pub workspace_root: PathBuf,
    pub relative_path: ResourcePath,
}

/// A validated Workspace-relative shared file interpreted by a merge adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FilesystemFileDescriptor {
    pub workspace_root: PathBuf,
    pub relative_path: ResourcePath,
    pub ownership_relative_path: ResourcePath,
}

/// Closed set of versioned Resource descriptors that Core stores but does not interpret.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "descriptor", rename_all = "snake_case")]
pub enum VersionedResourceDescriptor {
    FilesystemDirectoryV1(FilesystemDirectoryDescriptor),
    FilesystemFileV1(FilesystemFileDescriptor),
}

/// Lifecycle of one independently mutable external Resource.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceLifecycle {
    Active,
    Retiring,
}

/// External mutation, observation, locking, ownership, and recovery boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectResource {
    pub identity: EffectResourceId,
    pub scope: EffectScopeId,
    pub resource_key: ResourceKey,
    pub adapter: ResourceAdapterIdentity,
    pub descriptor: VersionedResourceDescriptor,
    pub format: MaterializationFormat,
    pub lifecycle: ResourceLifecycle,
}

/// Versioned contract that a Resource adapter uses to materialize Effects.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct MaterializationContract {
    pub kind: String,
    pub version: u32,
}

impl MaterializationContract {
    /// Builds the first filesystem Skill directory materialization contract.
    pub fn skill_directory_v1() -> Self {
        Self {
            kind: "ora/skill-directory".to_string(),
            version: 1,
        }
    }

    /// Returns the capability key used to match declarations with planners.
    pub fn capability_key(&self) -> String {
        format!("{}.v{}", self.kind, self.version)
    }
}

/// Versioned contract interpreted by the Consumer adapter during coordination.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct CoordinationContract {
    pub kind: String,
    pub version: u32,
}

impl CoordinationContract {
    /// Builds the Agent idle-barrier and restart contract.
    pub fn agent_restart_v1() -> Self {
        Self {
            kind: "ora/agent-restart".to_string(),
            version: 1,
        }
    }

    /// Returns the capability key used to validate Consumer declarations.
    pub fn capability_key(&self) -> String {
        format!("{}.v{}", self.kind, self.version)
    }
}

/// Coordination state is explicit so callers cannot pass an ambiguous boolean policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "contract", rename_all = "snake_case")]
pub enum CoordinationRequirement {
    Uninterrupted,
    QuiesceBeforeMutation(CoordinationContract),
}

/// One Target's declared relationship to one independently shared Resource.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetResourceBinding {
    pub target: EffectTargetId,
    pub resource: EffectResourceId,
    /// Older v1 declarations omitted this field because Skill was the only materialization.
    #[serde(default = "MaterializationContract::skill_directory_v1")]
    pub materialization_contract: MaterializationContract,
    pub accepts: CapabilityRequirement,
    pub coordination: CoordinationRequirement,
}

/// Complete replaceable Target declaration for one Scope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetDeclaration {
    pub target: EffectTargetId,
    pub consumer_revision: ConsumerRevisionId,
    pub bindings: BTreeMap<EffectResourceId, TargetResourceBinding>,
    pub digest: Digest,
}

/// Scope-independent Resource template published in one Consumer Revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FilesystemResourceTemplate {
    pub relative_path: ResourcePath,
    pub materialization_format: MaterializationFormat,
    pub materialization_contract: MaterializationContract,
    pub accepts: CapabilityRequirement,
    pub coordination: CoordinationRequirement,
    pub ownership_relative_path: Option<ResourcePath>,
}

impl FilesystemResourceTemplate {
    /// Produces the adapter-normalized physical key used to merge shared declarations in a Scope.
    pub fn resource_key(&self) -> ResourceKey {
        let resource_kind = if self.ownership_relative_path.is_some() {
            "filesystem-file"
        } else {
            "filesystem-directory"
        };
        ResourceKey::from_normalized(format!("{resource_kind}:{}", self.relative_path.as_str()))
    }
}

/// Complete Consumer declaration captured from one immutable runtime registration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConsumerDeclaration {
    pub consumer: ConsumerIdentity,
    pub adapter: ConsumerAdapterIdentity,
    pub capabilities: CapabilitySet,
    pub resources: Vec<FilesystemResourceTemplate>,
}

/// Reports unsafe or ambiguous Consumer/Resource declarations.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DeclarationError {
    #[error("unsafe Workspace-relative Resource path: {0}")]
    UnsafeRelativePath(String),
    #[error("the Workspace root cannot be an Effect Resource")]
    WorkspaceRootResource,
    #[error("materialization format must not be empty")]
    EmptyMaterializationFormat,
    #[error("Consumer lacks declared coordination capability {0}")]
    MissingCoordinationCapability(String),
    #[error("Consumer lacks declared materialization capability {0}")]
    MissingMaterializationCapability(String),
    #[error("one Consumer declaration assigns incompatible formats to Resource {0}")]
    IncompatibleResourceFormats(ResourceKey),
    #[error("one Consumer declaration assigns incompatible contracts to Resource {0}")]
    IncompatibleResourceContracts(ResourceKey),
}

impl ConsumerDeclaration {
    /// Validates capabilities and duplicate Resource keys before a declaration reaches persistence.
    pub fn validate(&self) -> Result<(), DeclarationError> {
        let mut resources = BTreeMap::new();
        for resource in &self.resources {
            let contract = resource.materialization_contract.capability_key();
            if !self
                .capabilities
                .materialization_contracts
                .contains(&contract)
            {
                return Err(DeclarationError::MissingMaterializationCapability(contract));
            }
            if let CoordinationRequirement::QuiesceBeforeMutation(contract) = &resource.coordination
                && !self
                    .capabilities
                    .coordination_contracts
                    .contains(&contract.capability_key())
            {
                return Err(DeclarationError::MissingCoordinationCapability(
                    contract.capability_key(),
                ));
            }
            let resource_key = resource.resource_key();
            if let Some((existing_format, existing_contract)) = resources.insert(
                resource_key.clone(),
                (
                    resource.materialization_format.clone(),
                    resource.materialization_contract.clone(),
                ),
            ) {
                if existing_format != resource.materialization_format {
                    return Err(DeclarationError::IncompatibleResourceFormats(resource_key));
                }
                if existing_contract != resource.materialization_contract {
                    return Err(DeclarationError::IncompatibleResourceContracts(
                        resource_key,
                    ));
                }
            }
        }
        Ok(())
    }
}
