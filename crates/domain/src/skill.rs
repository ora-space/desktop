use crate::{AuditFields, DomainModelError, Namespace, SkillId};
use serde::{Deserialize, Serialize};

/// Represents one reusable skill definition available to configurable agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub id: SkillId,
    pub namespace: Namespace,
    pub name: String,
    pub description: String,
    pub audit_fields: AuditFields,
}

impl Skill {
    /// Creates a skill while normalizing and validating its user-facing fields.
    pub fn new(
        id: SkillId,
        namespace: Namespace,
        name: impl Into<String>,
        description: impl Into<String>,
        audit_fields: AuditFields,
    ) -> Result<Self, DomainModelError> {
        let name = name.into().trim().to_string();
        let description = description.into().trim().to_string();

        validate_skill_name(&name).map_err(|error| match error {
            SkillNameError::Blank => DomainModelError::EmptySkillName,
            SkillNameError::Invalid => DomainModelError::InvalidSkillName { name: name.clone() },
            SkillNameError::TooLong => DomainModelError::SkillNameTooLong,
        })?;
        validate_skill_description(&description).map_err(|error| match error {
            SkillDescriptionError::Blank => DomainModelError::EmptySkillDescription,
            SkillDescriptionError::TooLarge => DomainModelError::SkillDescriptionTooLarge,
        })?;

        Ok(Self {
            id,
            namespace,
            name,
            description,
            audit_fields,
        })
    }
}

/// Reports why one user-facing skill name failed domain validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillNameError {
    Blank,
    Invalid,
    TooLong,
}

/// Reports why one user-facing skill description failed domain validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillDescriptionError {
    Blank,
    TooLarge,
}

/// Name of the reserved directory holding in-flight transaction staging.
pub const STAGING_DIR_NAME: &str = ".ora-staging";
/// Name of the reserved directory holding transaction compensation backups.
pub const BACKUP_DIR_NAME: &str = ".ora-backup";
/// Name of the reserved directory holding transaction journal markers.
pub const JOURNAL_DIR_NAME: &str = ".ora-journal";

/// Validates a trimmed skill name against the ASCII slug rules shared by every write path.
///
/// The name must be a single filesystem-safe path segment composed only of `A-Z`, `a-z`,
/// `0-9`, `.`, `_`, and `-`, and must not start with `.`. Rejecting every dot-prefixed name
/// (rather than only the reserved transaction directories) keeps the skills root disjoint from
/// hidden and reserved directories on any filesystem without needing a maintained allow/deny
/// list. The same byte and UTF-16 code-unit segment limits that protect archive paths also apply
/// so the name can always back a directory entry.
pub fn validate_skill_name(name: &str) -> Result<(), SkillNameError> {
    if name.is_empty() {
        return Err(SkillNameError::Blank);
    }
    if name.starts_with('.') {
        return Err(SkillNameError::Invalid);
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(SkillNameError::Invalid);
    }
    if name.len() > 255 || name.encode_utf16().count() > 255 {
        return Err(SkillNameError::TooLong);
    }
    Ok(())
}

/// Validates a trimmed skill description that must be non-empty and fit 4096 UTF-8 bytes.
pub fn validate_skill_description(description: &str) -> Result<(), SkillDescriptionError> {
    if description.is_empty() {
        return Err(SkillDescriptionError::Blank);
    }
    if description.len() > 4096 {
        return Err(SkillDescriptionError::TooLarge);
    }
    Ok(())
}
