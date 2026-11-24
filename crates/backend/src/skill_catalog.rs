#[cfg(test)]
mod tests;

use ora_agent_runtime::{SkillCatalogEntry, SkillCatalogSource};
use ora_application::{FilesystemSkillStorage, SkillRepository as _, skill_package_is_usable};
use ora_db::{RepositoryPool, SqliteSkillRepository};
use ora_domain::{SkillOrigin, WorkspaceId};
use ora_logging::ora_warn;
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
}

impl DesktopSkillCatalog {
    /// Creates a catalog reader backed by the shared SQLite pool and Desktop storage roots.
    pub(crate) fn new(pool: RepositoryPool, skills_root: PathBuf, home_directory: &Path) -> Self {
        Self {
            skill_repository: SqliteSkillRepository::new(pool),
            skills_root,
            plugins_installed_root: ora_plugin_manager::installed_root(home_directory),
        }
    }
}

impl SkillCatalogSource for DesktopSkillCatalog {
    fn skill_catalog(&self, _workspace_id: &WorkspaceId) -> Vec<SkillCatalogEntry> {
        let skills = match self.skill_repository.list_skills() {
            Ok(skills) => skills,
            Err(error) => {
                ora_warn!(
                    error = %error,
                    "failed to query skill catalog for session delivery"
                );
                return Vec::new();
            }
        };
        let storage = FilesystemSkillStorage::new(self.skills_root.clone());
        skills
            .into_iter()
            .filter_map(|skill| {
                match skill_package_is_usable(&storage, &skill) {
                    Ok(true) => {}
                    Ok(false) => return None,
                    Err(error) => {
                        ora_warn!(
                            skill_id = %skill.id,
                            skill_name = %skill.name,
                            error = %error,
                            "skipping unusable skill from session catalog"
                        );
                        return None;
                    }
                }
                let package_dir = match &skill.origin {
                    SkillOrigin::Local => self.skills_root.join(&skill.name),
                    SkillOrigin::Plugin { package_root, .. } => package_root.clone(),
                };
                Some(SkillCatalogEntry {
                    name: skill.name,
                    description: skill.description,
                    skill_md_path: package_dir.join("SKILL.md"),
                    package_dir,
                })
            })
            .collect()
    }

    fn extra_read_roots(&self) -> Vec<PathBuf> {
        vec![
            self.skills_root.clone(),
            self.plugins_installed_root.clone(),
        ]
    }
}
