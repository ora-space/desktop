use ora_contracts::{Skill as ContractSkill, SkillAvailability, SkillDetails, SkillSource};
use ora_domain::{Skill as DomainSkill, SkillOrigin};

/// Projects a domain skill into its audit-free public contract form.
pub(crate) fn map_skill(skill: DomainSkill, availability: SkillAvailability) -> ContractSkill {
    ContractSkill {
        id: skill.id.to_string(),
        namespace: skill.namespace.to_string(),
        name: skill.name,
        description: skill.description,
        source: map_source(&skill.origin),
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
        source: map_source(&skill.origin),
        availability,
    }
}

fn map_source(origin: &SkillOrigin) -> SkillSource {
    match origin {
        SkillOrigin::Local => SkillSource::Local,
        SkillOrigin::Plugin { plugin_id, .. } => SkillSource::Plugin {
            plugin_id: plugin_id.canonical(),
        },
    }
}
