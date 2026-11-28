use ora_contracts::{
    PluginOrigin, Skill as ContractSkill, SkillAvailability, SkillDetails, SkillSource,
};
use ora_domain::{PluginOriginCatalog, Skill as DomainSkill, SkillOrigin};

/// Projects a domain skill into its audit-free public contract form.
pub(crate) fn map_skill(
    skill: DomainSkill,
    availability: SkillAvailability,
    origins: &PluginOriginCatalog,
) -> ContractSkill {
    ContractSkill {
        id: skill.id.to_string(),
        namespace: skill.namespace.to_string(),
        name: skill.name,
        description: skill.description,
        source: map_source(&skill.origin, origins),
        availability,
    }
}

/// Projects one skill together with the Markdown body loaded from formal storage.
pub(crate) fn map_skill_details(
    skill: DomainSkill,
    content: String,
    availability: SkillAvailability,
    origins: &PluginOriginCatalog,
) -> SkillDetails {
    SkillDetails {
        id: skill.id.to_string(),
        namespace: skill.namespace.to_string(),
        name: skill.name,
        description: skill.description,
        content,
        source: map_source(&skill.origin, origins),
        availability,
    }
}

/// Projects where a Skill came from; a plugin Skill inherits its plugin's origin.
fn map_source(origin: &SkillOrigin, origins: &PluginOriginCatalog) -> SkillSource {
    match origin {
        SkillOrigin::Local => SkillSource::Local,
        SkillOrigin::Plugin { plugin_id, .. } => SkillSource::Plugin {
            plugin_id: plugin_id.canonical(),
            origin: plugin_origin_contract(origins, plugin_id.namespace()),
        },
    }
}

/// Classifies one plugin namespace for a contract, degrading instead of failing on a lost binding.
///
/// The degrade-and-warn policy lives in [`PluginOriginCatalog::resolve_or_degrade`]; this only
/// projects the domain origin onto the wire enum.
pub fn plugin_origin_contract(origins: &PluginOriginCatalog, namespace: &str) -> PluginOrigin {
    match origins.resolve_or_degrade(namespace) {
        ora_domain::PluginOrigin::LocalArchive => PluginOrigin::LocalArchive,
        ora_domain::PluginOrigin::Official => PluginOrigin::Official,
        ora_domain::PluginOrigin::Marketplace { source_url, label } => {
            PluginOrigin::Marketplace { source_url, label }
        }
    }
}
