use crate::{AuditFields, DomainModelError, ProjectId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Identifies whether a workspace is the project's canonical checkout or an isolated checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceKind {
    Main,
    Isolated,
}

impl WorkspaceKind {
    /// Returns the stable text stored in SQLite for this workspace kind.
    pub fn database_value(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Isolated => "isolated",
        }
    }

    /// Converts a persisted workspace kind into the supported domain value.
    pub fn from_database_value(value: &str) -> Result<Self, DomainModelError> {
        match value {
            "main" => Ok(Self::Main),
            "isolated" => Ok(Self::Isolated),
            other => Err(DomainModelError::InvalidWorkspaceKind(other.to_string())),
        }
    }
}

/// Captures the durable lifecycle admission state of a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceLifecycle {
    Provisioning,
    Active,
    Unavailable,
    Retiring,
    Deleted,
}

impl WorkspaceLifecycle {
    /// Returns the stable text stored in SQLite for this lifecycle state.
    pub fn database_value(self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::Active => "active",
            Self::Unavailable => "unavailable",
            Self::Retiring => "retiring",
            Self::Deleted => "deleted",
        }
    }

    /// Converts a persisted lifecycle string into a typed state.
    pub fn from_database_value(value: &str) -> Result<Self, DomainModelError> {
        match value {
            "provisioning" => Ok(Self::Provisioning),
            "active" => Ok(Self::Active),
            "unavailable" => Ok(Self::Unavailable),
            "retiring" => Ok(Self::Retiring),
            "deleted" => Ok(Self::Deleted),
            other => Err(DomainModelError::InvalidWorkspaceLifecycle(
                other.to_string(),
            )),
        }
    }
}

/// Describes the physical or provider-backed location of a workspace.
///
/// Remote locations intentionally retain references and opaque locator data only. Credentials
/// are resolved by the adapter selected by the reference and never become part of this value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceLocation {
    LocalFilesystem {
        path: String,
    },
    Ssh {
        connection_ref: String,
        path: String,
    },
    RemoteTarget {
        plugin_id: String,
        target_ref: String,
        locator: String,
    },
}

impl WorkspaceLocation {
    /// Builds a local filesystem location from its path representation.
    pub fn local_filesystem(path: impl Into<String>) -> Self {
        Self::LocalFilesystem { path: path.into() }
    }

    /// Returns the location kind token used by the database adapter.
    pub fn database_kind(&self) -> &'static str {
        match self {
            Self::LocalFilesystem { .. } => "local_filesystem",
            Self::Ssh { .. } => "ssh",
            Self::RemoteTarget { .. } => "remote_target",
        }
    }
}

/// Identifies the mechanism responsible for creating or managing a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceProvisionerKind {
    LocalGit,
    Ssh,
    RemoteTarget,
}

impl WorkspaceProvisionerKind {
    /// Returns the stable text stored in SQLite for this provisioner.
    pub fn database_value(self) -> &'static str {
        match self {
            Self::LocalGit => "local_git",
            Self::Ssh => "ssh",
            Self::RemoteTarget => "remote_target",
        }
    }

    /// Converts a persisted provisioner token into a typed value.
    pub fn from_database_value(value: &str) -> Result<Self, DomainModelError> {
        match value {
            "local_git" => Ok(Self::LocalGit),
            "ssh" => Ok(Self::Ssh),
            "remote_target" => Ok(Self::RemoteTarget),
            other => Err(DomainModelError::InvalidWorkspaceProvisionerKind(
                other.to_string(),
            )),
        }
    }
}

/// Captures the durable progress of a workspace provisioning operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceProvisioningState {
    Pending,
    Provisioning,
    Ready,
    Failed,
    Destroying,
    Destroyed,
}

impl WorkspaceProvisioningState {
    /// Returns the stable text stored in SQLite for this state.
    pub fn database_value(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Provisioning => "provisioning",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Destroying => "destroying",
            Self::Destroyed => "destroyed",
        }
    }

    /// Converts a persisted provisioning state into a typed value.
    pub fn from_database_value(value: &str) -> Result<Self, DomainModelError> {
        match value {
            "pending" => Ok(Self::Pending),
            "provisioning" => Ok(Self::Provisioning),
            "ready" => Ok(Self::Ready),
            "failed" => Ok(Self::Failed),
            "destroying" => Ok(Self::Destroying),
            "destroyed" => Ok(Self::Destroyed),
            other => Err(DomainModelError::InvalidWorkspaceProvisioningState(
                other.to_string(),
            )),
        }
    }
}

/// Records desired and confirmed provisioning facts independently from workspace identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceProvisioning {
    pub workspace_id: WorkspaceId,
    pub provisioner_kind: WorkspaceProvisionerKind,
    pub plugin_id: Option<String>,
    pub requested_revision: Option<String>,
    pub requested_branch: Option<String>,
    pub actual_revision: Option<String>,
    pub actual_branch: Option<String>,
    pub requested_locator: Option<String>,
    pub actual_locator: Option<String>,
    pub state: WorkspaceProvisioningState,
    pub last_error_code: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Represents a stable execution environment shared by sessions and workflow runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub project_id: ProjectId,
    pub kind: WorkspaceKind,
    pub location: WorkspaceLocation,
    pub lifecycle: WorkspaceLifecycle,
    pub audit_fields: AuditFields,
}

impl Workspace {
    /// Creates a workspace snapshot with the caller-selected identity and lifecycle.
    pub fn new(
        id: WorkspaceId,
        project_id: ProjectId,
        kind: WorkspaceKind,
        location: WorkspaceLocation,
        lifecycle: WorkspaceLifecycle,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            project_id,
            kind,
            location,
            lifecycle,
            audit_fields,
        }
    }

    /// Reports whether this workspace may admit a new session or workflow run.
    pub fn is_admissible(&self) -> bool {
        self.lifecycle == WorkspaceLifecycle::Active && !self.audit_fields.is_deleted
    }
}
