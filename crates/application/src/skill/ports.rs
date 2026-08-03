use crate::RepositoryError;
use ora_domain::{Skill, SkillId};

/// Defines persistence operations required by the skill CRUD use cases.
pub trait SkillRepository {
    /// Persists a new skill snapshot.
    fn create_skill(&self, skill: Skill) -> Result<Skill, RepositoryError>;

    /// Loads one visible skill by identifier.
    fn find_skill(&self, skill_id: &SkillId) -> Result<Option<Skill>, RepositoryError>;

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

/// Supplies new skill identifiers for create use cases.
pub trait SkillIdGenerator {
    /// Produces the identifier for a newly created skill.
    fn generate_skill_id(&self) -> SkillId;
}
