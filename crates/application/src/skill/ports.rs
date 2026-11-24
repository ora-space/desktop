use crate::RepositoryError;
use ora_domain::{Namespace, Skill, SkillId};

/// Defines catalog persistence required by skill CRUD and import sessions.
pub trait SkillRepository {
    /// Persists a new visible skill snapshot.
    fn create_skill(&self, skill: Skill) -> Result<Skill, RepositoryError>;

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

    /// Marks a visible skill deleted at the supplied timestamp.
    fn soft_delete_skill(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError>;
}

/// Supplies opaque identifiers for newly created skills.
pub trait SkillIdGenerator {
    /// Produces an identifier that does not reveal a filesystem path.
    fn generate_skill_id(&self) -> SkillId;
}
