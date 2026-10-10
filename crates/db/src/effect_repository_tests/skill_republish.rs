use super::{fixture, package_fingerprint};
use crate::{
    PluginSkillProjection, SqliteEffectRepository, SqliteSkillRepository, test_clock::TestClock,
};
use ora_domain::PluginId;
use ora_effect::*;
use pretty_assertions::assert_eq;

/// Writes one Skill package directory and returns its validated projection.
fn skill_package(
    directory: &std::path::Path,
    package_name: &str,
    manifest: &[u8],
) -> PluginSkillProjection {
    let package_root = directory.join(package_name);
    std::fs::create_dir_all(&package_root)
        .unwrap_or_else(|error| panic!("create package: {error}"));
    std::fs::write(package_root.join("SKILL.md"), manifest)
        .unwrap_or_else(|error| panic!("write manifest: {error}"));
    PluginSkillProjection {
        name: "review".to_string(),
        description: "Reviews changes".to_string(),
        package_fingerprint: package_fingerprint(&package_root),
        package_root,
        skill_md_digest: Digest::sha256(manifest),
    }
}

/// Repeated install/uninstall of one plugin must keep the Skill catalog converging: retiring a
/// source removes its Desired intent, and re-publishing the same revision must restore it.
#[test]
fn reinstalling_the_same_plugin_revision_restores_desired_effects() {
    let (directory, pool, workspace) = fixture();
    let clock = TestClock::new(100);
    let skills = SqliteSkillRepository::with_clock(pool.clone(), clock.clone());
    let repository = SqliteEffectRepository::with_clock(pool.clone(), clock.clone());
    let scope = EffectScopeId::Workspace(workspace.id.clone());
    let plugin =
        PluginId::new("local", "ora-space.codeagent").unwrap_or_else(|e| panic!("plugin: {e}"));
    let package = skill_package(
        directory.path(),
        "skill-review",
        b"---\nname: review\ndescription: Reviews changes\n---\ncontent\n",
    );

    // Install: the Skill becomes Desired intent in the Workspace Scope.
    skills
        .replace_plugin_skills(&plugin, "0.6.0", &[package.clone()], 110)
        .unwrap_or_else(|e| panic!("install plugin Skills: {e}"));
    assert_eq!(
        repository
            .load_desired_state(&scope)
            .unwrap_or_else(|e| panic!("load Desired State: {e}"))
            .effects
            .len(),
        1
    );

    // Uninstall: the Skill disappears from the catalog and from Desired intent.
    skills
        .remove_plugin_skills(&plugin, 120)
        .unwrap_or_else(|e| panic!("uninstall plugin Skills: {e}"));
    assert_eq!(
        repository
            .load_desired_state(&scope)
            .unwrap_or_else(|e| panic!("load Desired State: {e}"))
            .effects
            .len(),
        0
    );

    // Reinstall the exact same revision: the Skill must become Desired intent again so a
    // re-paired Target can converge instead of waiting for intent that never returns.
    skills
        .replace_plugin_skills(&plugin, "0.6.0", &[package], 130)
        .unwrap_or_else(|e| panic!("reinstall plugin Skills: {e}"));
    assert_eq!(
        repository
            .load_desired_state(&scope)
            .unwrap_or_else(|e| panic!("load Desired State: {e}"))
            .effects
            .len(),
        1
    );
}
