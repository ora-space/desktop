use crate::RepositoryError;
use ora_domain::{Namespace, Skill, SkillId};
use ora_effect::Digest;
use std::path::PathBuf;

/// Exact validated Local source metadata committed with its catalog row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalSkillSourceRevision {
    pub skill_md_digest: Digest,
    pub package_root: PathBuf,
}

/// Defines catalog persistence required by skill CRUD and import sessions.
pub trait SkillRepository {
    /// Persists a new visible skill snapshot.
    fn create_skill(&self, skill: Skill) -> Result<Skill, RepositoryError>;

    /// Persists a new catalog row and its active Local source state atomically.
    fn create_skill_with_source(
        &self,
        skill: Skill,
        _source: LocalSkillSourceRevision,
    ) -> Result<Skill, RepositoryError> {
        self.create_skill(skill)
    }

    /// Loads one visible skill by its stable identifier.
    fn find_skill(&self, skill_id: &SkillId) -> Result<Option<Skill>, RepositoryError>;

    /// Loads one visible skill by namespace and ASCII case-insensitive name.
    fn find_skill_by_name(
        &self,
        namespace: &Namespace,
        name: &str,
    ) -> Result<Option<Skill>, RepositoryError>;

    /// Lists visible skills in deterministic storage order.
    fn list_skills(&self) -> Result<Vec<Skill>, RepositoryError>;

    /// Replaces a visible skill identified by its stable identifier.
    fn update_skill(&self, skill: Skill) -> Result<Skill, RepositoryError>;

    /// Updates a catalog row, source state, and durable propagation event atomically.
    fn update_skill_with_source(
        &self,
        skill: Skill,
        _source: LocalSkillSourceRevision,
    ) -> Result<Skill, RepositoryError> {
        self.update_skill(skill)
    }

    /// Marks a visible skill deleted at the supplied timestamp.
    fn soft_delete_skill(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError>;

    /// Soft-deletes a Local source and uninstalls it from every Workspace atomically.
    fn soft_delete_skill_with_source(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
        self.soft_delete_skill(skill_id, deleted_at)
    }
}

/// Supplies opaque identifiers for newly created skills.
pub trait SkillIdGenerator {
    /// Produces an identifier that does not reveal a filesystem path.
    fn generate_skill_id(&self) -> SkillId;
}
