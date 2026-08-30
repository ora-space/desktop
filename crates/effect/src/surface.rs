use crate::{ConsumerId, SurfaceKey};
use ora_domain::WorkspaceId;
use ora_utils::path::PortableRelativePath;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use thiserror::Error;

/// A normalized, safe Workspace-relative surface path.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurfacePath(PortableRelativePath);

impl SurfacePath {
    /// Parses a consumer declaration and refuses the Workspace root itself.
    pub fn parse(value: &str) -> Result<Self, DescriptorMergeError> {
        let path = PortableRelativePath::parse(value)
            .map_err(|_| DescriptorMergeError::UnsafeRelativePath(value.to_string()))?;
        if path.is_root() {
            return Err(DescriptorMergeError::WorkspaceRootSurface);
        }
        Ok(Self(path))
    }

    /// Returns the normalized slash-separated persistence representation.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Reconstructs the path with host-native components.
    pub fn to_path_buf(&self) -> std::path::PathBuf {
        self.0.to_path_buf()
    }
}

impl Display for SurfacePath {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for SurfacePath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SurfacePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

/// Identifies an adapter-compatible on-disk representation.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct MaterializationFormat(String);

impl MaterializationFormat {
    /// Returns the first-version directory-tree format used for Skill packages.
    pub fn skill_directory_v1() -> Self {
        Self("skill_directory.v1".to_string())
    }

    /// Returns the complete-file format the Ora-owned OpenCode MCP config is rendered into.
    ///
    /// A distinct format identifier is what dispatches an MCP surface to its own adapter rather
    /// than the Skill reconciler: the worker branches on this value, so an MCP desired row can
    /// never reach the Skill planner.
    pub fn opencode_mcp_complete_file_v1() -> Self {
        Self("opencode_mcp_complete_file.v1".to_string())
    }

    /// Builds a named format for plugin adapters and compatibility tests.
    pub fn named(value: impl Into<String>) -> Result<Self, DescriptorMergeError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(DescriptorMergeError::EmptyMaterializationFormat);
        }
        Ok(Self(value))
    }

    /// Returns the stable adapter format identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Selects how filesystem mutation must coordinate with one surface consumer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerCoordination {
    Uninterrupted,
    WaitForIdleAndRestart,
}

/// One consumer's data-only declaration of a filesystem surface, dispatched by materialization
/// format.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FilesystemSkillSurface {
    pub workspace_relative_path: SurfacePath,
    pub materialization_format: MaterializationFormat,
    pub consumer: ConsumerId,
    pub coordination: ConsumerCoordination,
}

/// The format-polymorphic view of one consumer's surface declaration.
///
/// Both Skill and MCP declarations satisfy this trait so [`SurfaceDescriptorSet::merge`] groups
/// either kind by physical path without knowing which adapter will reconcile it. The kind is
/// decided later, at the `format_kind` the descriptor carries — one adapter per format, so an MCP
/// desired row never enters the Skill reconciler and a Skill row never reaches the MCP adapter.
pub trait SurfaceDeclaration {
    fn workspace_relative_path(&self) -> &SurfacePath;
    fn materialization_format(&self) -> &MaterializationFormat;
    fn consumer(&self) -> &ConsumerId;
    fn coordination(&self) -> ConsumerCoordination;
}

impl SurfaceDeclaration for FilesystemSkillSurface {
    fn workspace_relative_path(&self) -> &SurfacePath {
        &self.workspace_relative_path
    }
    fn materialization_format(&self) -> &MaterializationFormat {
        &self.materialization_format
    }
    fn consumer(&self) -> &ConsumerId {
        &self.consumer
    }
    fn coordination(&self) -> ConsumerCoordination {
        self.coordination
    }
}

/// The canonical Workspace-relative path of the Ora-owned OpenCode MCP complete file.
///
/// The OpenCode Agent declares this exact path for its MCP surface, so the read path resolves the
/// surface key for one workspace from it without a separate surface-descriptor load. It is the
/// path the [`FilesystemMcpSurface`] the OpenCode adapter registers carries.
pub const OPENCODE_MCP_COMPLETE_FILE_RELATIVE_PATH: &str = ".opencode/opencode.jsonc";

/// One consumer's declaration of the Ora-owned complete MCP file surface.
///
/// The physical path is the Workspace-relative file the adapter owns (e.g.
/// `.opencode/opencode.jsonc`); the format dispatches the surface to the MCP adapter rather than
/// the Skill reconciler. Consumers and coordination match the Skill surface shape so the shared
/// merge, status, and consumer infrastructure is reused without a parallel surface table.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FilesystemMcpSurface {
    pub workspace_relative_path: SurfacePath,
    pub materialization_format: MaterializationFormat,
    pub consumer: ConsumerId,
    pub coordination: ConsumerCoordination,
}

impl SurfaceDeclaration for FilesystemMcpSurface {
    fn workspace_relative_path(&self) -> &SurfacePath {
        &self.workspace_relative_path
    }
    fn materialization_format(&self) -> &MaterializationFormat {
        &self.materialization_format
    }
    fn consumer(&self) -> &ConsumerId {
        &self.consumer
    }
    fn coordination(&self) -> ConsumerCoordination {
        self.coordination
    }
}

/// Persisted lifecycle of a physical surface after consumer changes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceLifecycle {
    Active,
    Retiring,
}

/// Merged physical surface plus the consumers sharing it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceDescriptorSet {
    pub surface_key: SurfaceKey,
    pub path: SurfacePath,
    pub format: MaterializationFormat,
    pub consumers: BTreeMap<ConsumerId, ConsumerCoordination>,
    pub lifecycle: SurfaceLifecycle,
}

impl SurfaceDescriptorSet {
    /// Merges compatible descriptors so one physical path has exactly one reconciler ledger.
    ///
    /// Accepts any surface declaration (Skill or MCP) via the [`SurfaceDeclaration`] trait; two
    /// declarations at the same path must agree on format, which is what keeps the Skill and MCP
    /// adapters from being claimed for one path at once.
    pub fn merge(
        workspace_id: &WorkspaceId,
        descriptors: impl IntoIterator<Item = impl SurfaceDeclaration>,
    ) -> Result<Vec<Self>, DescriptorMergeError> {
        let mut grouped: BTreeMap<
            SurfacePath,
            (
                MaterializationFormat,
                BTreeMap<ConsumerId, ConsumerCoordination>,
            ),
        > = BTreeMap::new();
        for descriptor in descriptors {
            let entry = grouped
                .entry(descriptor.workspace_relative_path().clone())
                .or_insert_with(|| (descriptor.materialization_format().clone(), BTreeMap::new()));
            if &entry.0 != descriptor.materialization_format() {
                return Err(DescriptorMergeError::IncompatibleSurfaceDeclarations {
                    path: descriptor.workspace_relative_path().clone(),
                    first_format: entry.0.clone(),
                    second_format: descriptor.materialization_format().clone(),
                });
            }
            entry
                .1
                .insert(descriptor.consumer().clone(), descriptor.coordination());
        }

        Ok(grouped
            .into_iter()
            .map(|(path, (format, consumers))| Self {
                surface_key: SurfaceKey::for_workspace(workspace_id, path.as_str()),
                path,
                format,
                consumers,
                lifecycle: SurfaceLifecycle::Active,
            })
            .collect())
    }

    /// Returns whether at least one consumer requires session quiescence before mutation.
    pub fn requires_coordination(&self) -> bool {
        self.consumers
            .values()
            .any(|policy| *policy == ConsumerCoordination::WaitForIdleAndRestart)
    }
}

/// Reports an invalid or mutually incompatible consumer surface declaration.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DescriptorMergeError {
    #[error("unsafe Workspace-relative surface path: {0}")]
    UnsafeRelativePath(String),
    #[error("the Workspace root cannot be a Skill surface")]
    WorkspaceRootSurface,
    #[error("materialization format must not be empty")]
    EmptyMaterializationFormat,
    #[error("incompatible surface declarations at {path}")]
    IncompatibleSurfaceDeclarations {
        path: SurfacePath,
        first_format: MaterializationFormat,
        second_format: MaterializationFormat,
    },
}
