use ora_contracts::{Skill as ContractSkill, SkillAvailability, SkillDetails};
use ora_domain::Skill as DomainSkill;

/// Projects a domain skill into its audit-free public contract form.
pub(crate) fn map_skill(skill: DomainSkill, availability: SkillAvailability) -> ContractSkill {
    ContractSkill {
        id: skill.id.to_string(),
        namespace: skill.namespace.to_string(),
        name: skill.name,
        description: skill.description,
        availability,
    }
}

/// Projects one skill together with the Markdown body loaded from formal storage.
pub(crate) fn map_skill_details(
    skill: DomainSkill,
    content: String,
    availability: SkillAvailability,
) -> SkillDetails {
    SkillDetails {
        id: skill.id.to_string(),
        namespace: skill.namespace.to_string(),
        name: skill.name,
        description: skill.description,
        content,
        availability,
    }
}
