use ora_contracts::{
    PluginOrigin, Skill as ContractSkill, SkillAvailability, SkillDetails, SkillSource,
};
use ora_domain::{PluginOriginCatalog, Skill as DomainSkill, SkillOrigin};
use ora_logging::ora_warn;

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
/// One damaged binding must not fail a whole list, so the entry is shown with the degraded origin
/// and the damage is left in the log for diagnosis.
pub fn plugin_origin_contract(origins: &PluginOriginCatalog, namespace: &str) -> PluginOrigin {
    let origin = origins.origin_of(namespace).unwrap_or_else(|missing| {
        ora_warn!(
            namespace = missing.namespace(),
            "plugin namespace has no recorded marketplace source"
        );
        missing.into_degraded_origin()
    });
    match origin {
        ora_domain::PluginOrigin::LocalArchive => PluginOrigin::LocalArchive,
        ora_domain::PluginOrigin::Official => PluginOrigin::Official,
        ora_domain::PluginOrigin::Marketplace { source_url, label } => {
            PluginOrigin::Marketplace { source_url, label }
        }
    }
}
