use ora_domain::{Skill, SkillId};
use thiserror::Error;

/// Defines persistence operations required by the skill CRUD use cases.
pub trait SkillRepository {
    /// Persists a new skill snapshot.
    fn create_skill(&self, skill: Skill) -> Result<Skill, SkillRepositoryError>;

    /// Loads one visible skill by identifier.
    fn find_skill(&self, skill_id: &SkillId) -> Result<Option<Skill>, SkillRepositoryError>;

    /// Loads one visible skill by name using ASCII case-insensitive comparison.
    fn find_skill_by_name(&self, name: &str) -> Result<Option<Skill>, SkillRepositoryError>;

    /// Lists visible skills in deterministic storage order.
    fn list_skills(&self) -> Result<Vec<Skill>, SkillRepositoryError>;

    /// Replaces a visible skill identified by its stable identifier.
    fn update_skill(&self, skill: Skill) -> Result<Skill, SkillRepositoryError>;

    /// Marks a visible skill deleted at the supplied timestamp.
    fn soft_delete_skill(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<bool, SkillRepositoryError>;
}

/// Supplies new skill identifiers for create use cases.
pub trait SkillIdGenerator {
    /// Produces the identifier for a newly created skill.
    fn generate_skill_id(&self) -> SkillId;
}

/// Represents storage failures exposed as stable application outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillRepositoryError {
    #[error("skill repository operation failed: {0}")]
    OperationFailed(String),
}
