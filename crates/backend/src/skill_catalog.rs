#[cfg(test)]
mod tests;

use crate::skill_revision::SkillRevision;
use ora_agent_runtime::{
    RuntimeError, SkillCatalogEntry, SkillCatalogSnapshot, SkillCatalogSource,
};
use ora_application::{FilesystemSkillStorage, SkillRepository as _, skill_package_is_usable};
use ora_db::{RepositoryPool, SqliteSkillRepository};
use ora_domain::{Skill, SkillOrigin, WorkspaceId};
use ora_logging::ora_warn;
use ora_utils::path::strip_verbatim_prefix;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Resolves live Skill catalog entries and host read roots directly from Ora's formal storage.
///
/// Local skills live under `<app_data>/atoms/skills/<name>` while plugin skills point directly
/// into their installed package directories under `<home>/plugins/installed`.
#[derive(Clone)]
pub(crate) struct DesktopSkillCatalog {
    skill_repository: SqliteSkillRepository,
    skills_root: PathBuf,
    plugins_installed_root: PathBuf,
    revision: SkillRevision,
}

impl DesktopSkillCatalog {
    /// Creates a catalog reader backed by the shared SQLite pool and Desktop storage roots.
    pub(crate) fn new(
        pool: RepositoryPool,
        skills_root: PathBuf,
        home_directory: &Path,
        revision: SkillRevision,
    ) -> Self {
        Self {
            skill_repository: SqliteSkillRepository::new(pool),
            skills_root: strip_verbatim_prefix(&skills_root),
            plugins_installed_root: strip_verbatim_prefix(&ora_plugin_manager::installed_root(
                home_directory,
            )),
            revision,
        }
    }
}

/// Where a Skill's package lives, as both the sessions and the settings see it.
///
/// Both readers report what they found available as these paths (see `SkillRevision::observed`), so
/// the same Skills read by either one must give the same set, or each read would count a change.
pub(crate) fn skill_package_dir(skills_root: &Path, skill: &Skill) -> PathBuf {
    match &skill.origin {
        SkillOrigin::Local => skills_root.join(&skill.name),
        SkillOrigin::Plugin { package_root, .. } => strip_verbatim_prefix(package_root),
    }
}

impl SkillCatalogSource for DesktopSkillCatalog {
    fn skill_catalog(
        &self,
        _workspace_id: &WorkspaceId,
    ) -> Result<SkillCatalogSnapshot, RuntimeError> {
        // Read first: a change that lands while the packages are read is counted on its own.
        let revision = self.revision.current();
        let skills = self.skill_repository.list_skills().map_err(|error| {
            RuntimeError::internal("failed to query skill catalog for session delivery", error)
        })?;
        let storage = FilesystemSkillStorage::new(self.skills_root.clone());
        let mut entries = Vec::with_capacity(skills.len());
        let mut unreadable_paths = BTreeSet::new();
        for skill in skills {
            let package_dir = skill_package_dir(&self.skills_root, &skill);
            let skill_md_path = package_dir.join("SKILL.md");
            match skill_package_is_usable(&storage, &skill) {
                Ok(true) => {
                    entries.push(SkillCatalogEntry {
                        name: skill.name,
                        description: skill.description,
                        skill_md_path,
                        package_dir,
                    });
                }
                Ok(false) => {}
                Err(error) => {
                    ora_warn!(
                        skill_id = %skill.id,
                        skill_name = %skill.name,
                        path = %skill_md_path.display(),
                        error = %error,
                        "failed to read skill package manifest; treating skill availability as unknown",
                    );
                    unreadable_paths.insert(skill_md_path);
                }
            }
        }
        self.revision.observed(
            revision,
            entries
                .iter()
                .map(|entry| entry.skill_md_path.clone())
                .collect(),
        );
        Ok(SkillCatalogSnapshot {
            entries,
            unreadable_paths,
        })
    }

    fn revision(&self) -> u64 {
        self.revision.current()
    }

    fn extra_read_roots(&self) -> Vec<PathBuf> {
        vec![
            self.skills_root.clone(),
            self.plugins_installed_root.clone(),
        ]
    }
}
