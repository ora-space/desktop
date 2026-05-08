use crate::{ArtifactId, AuditFields, DomainModelError, VirtualEntryId, VirtualFolderId};
use serde::{Deserialize, Serialize};

/// Distinguishes file entries from directory entries in a virtual folder tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VirtualEntryKind {
    File,
    Directory,
}

impl VirtualEntryKind {
    /// Returns the integer code used by persistence adapters for this entry kind.
    pub fn database_value(self) -> i64 {
        match self {
            Self::File => 0,
            Self::Directory => 1,
        }
    }

    /// Converts a persisted integer into a strongly typed virtual entry kind.
    pub fn from_database_value(value: i64) -> Result<Self, DomainModelError> {
        match value {
            0 => Ok(Self::File),
            1 => Ok(Self::Directory),
            _ => Err(DomainModelError::InvalidVirtualEntryKind(value)),
        }
    }
}

impl TryFrom<i64> for VirtualEntryKind {
    type Error = DomainModelError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::from_database_value(value)
    }
}

/// Represents one file-system-like node within a virtual folder tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualEntry {
    pub id: VirtualEntryId,
    pub virtual_folder_id: VirtualFolderId,
    pub parent_entry_id: Option<VirtualEntryId>,
    pub name: String,
    pub kind: VirtualEntryKind,
    pub content_ref: Option<ArtifactId>,
    pub audit_fields: AuditFields,
}

impl VirtualEntry {
    /// Creates a virtual entry snapshot together with its persistence-managed audit metadata.
    pub fn new(
        id: VirtualEntryId,
        virtual_folder_id: VirtualFolderId,
        parent_entry_id: Option<VirtualEntryId>,
        name: impl Into<String>,
        kind: VirtualEntryKind,
        content_ref: Option<ArtifactId>,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            virtual_folder_id,
            parent_entry_id,
            name: name.into(),
            kind,
            content_ref,
            audit_fields,
        }
    }
}
