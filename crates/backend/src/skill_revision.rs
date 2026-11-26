//! Tells which discoveries were made before the Skills last changed.

use ora_application::{RepositoryError, SkillRepository};
use ora_db::{DatabaseError, PluginSkillProjection, SqliteSkillRepository};
use ora_domain::{Namespace, PluginId, Skill, SkillId};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

#[cfg(test)]
mod tests;

/// Counts the changes to the Skills this process serves, and announces each one.
///
/// A count rather than a digest of the catalog: a catalog that returns to an earlier state, or whose
/// packages changed behind the same names, must not match what was discovered before.
#[derive(Clone, Default)]
pub(crate) struct SkillRevision {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    announce: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

#[derive(Default)]
struct State {
    revision: u64,
    /// The `SKILL.md` paths available when last looked at since the revision advanced.
    available: Option<BTreeSet<PathBuf>>,
}

impl SkillRevision {
    /// Names who is told about each change. Earlier changes have nobody to tell.
    pub(crate) fn announce_with(&self, announce: impl Fn() + Send + Sync + 'static) {
        let _ = self.inner.announce.set(Box::new(announce));
    }

    /// Returns the current revision, which only ever grows.
    pub(crate) fn current(&self) -> u64 {
        self.lock().revision
    }

    /// Advances after one catalog change was committed, whatever it left the catalog at.
    pub(crate) fn committed(&self) {
        self.advance(&mut self.lock());
        self.announce();
    }

    /// Advances when the available Skills, read at `revision`, are not those last read.
    ///
    /// Availability also depends on package files and plugins, whose changes commit nothing. The
    /// first look after a commit only says what that commit left.
    pub(crate) fn observed(&self, revision: u64, available: BTreeSet<PathBuf>) {
        {
            let mut state = self.lock();
            // Read before a change that has been counted meanwhile.
            if state.revision != revision {
                return;
            }
            match state.available.replace(available) {
                Some(previous) if state.available.as_ref() != Some(&previous) => {
                    let available = state.available.take();
                    self.advance(&mut state);
                    state.available = available;
                }
                _ => return,
            }
        }
        self.announce();
    }

    /// Counts one change, after which nothing has been looked at yet.
    fn advance(&self, state: &mut State) {
        state.revision += 1;
        state.available = None;
    }

    /// Tells whoever listens, outside the lock, because it may come back for the revision.
    fn announce(&self) {
        if let Some(announce) = self.inner.announce.get() {
            announce();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// The Skill catalog as the code that changes it sees it: every committed change is counted.
#[derive(Clone)]
pub(crate) struct CountedSkillRepository {
    repository: SqliteSkillRepository,
    revision: SkillRevision,
}

impl CountedSkillRepository {
    pub(crate) fn new(repository: SqliteSkillRepository, revision: SkillRevision) -> Self {
        Self {
            repository,
            revision,
        }
    }

    /// Replaces the catalog projection owned by one installed Skill plugin.
    pub(crate) fn replace_plugin_skills(
        &self,
        plugin_id: &PluginId,
        plugin_version: &str,
        skills: &[PluginSkillProjection],
        updated_at: i64,
    ) -> Result<(), DatabaseError> {
        let changed =
            self.repository
                .replace_plugin_skills(plugin_id, plugin_version, skills, updated_at)?;
        self.count(changed);
        Ok(())
    }

    /// Makes every Skill from an uninstalled plugin disappear from the catalog.
    pub(crate) fn remove_plugin_skills(
        &self,
        plugin_id: &PluginId,
        updated_at: i64,
    ) -> Result<(), DatabaseError> {
        let changed = self
            .repository
            .remove_plugin_skills(plugin_id, updated_at)?;
        self.count(changed);
        Ok(())
    }

    /// Counts a commit, unless it left the catalog alone.
    fn count(&self, changed: bool) {
        if changed {
            self.revision.committed();
        }
    }
}

impl SkillRepository for CountedSkillRepository {
    fn create_skill(&self, skill: Skill) -> Result<Skill, RepositoryError> {
        let created = self.repository.create_skill(skill)?;
        self.count(true);
        Ok(created)
    }

    fn find_skill(&self, skill_id: &SkillId) -> Result<Option<Skill>, RepositoryError> {
        self.repository.find_skill(skill_id)
    }

    fn find_skill_by_name(
        &self,
        namespace: &Namespace,
        name: &str,
    ) -> Result<Option<Skill>, RepositoryError> {
        self.repository.find_skill_by_name(namespace, name)
    }

    fn list_skills(&self) -> Result<Vec<Skill>, RepositoryError> {
        self.repository.list_skills()
    }

    fn update_skill(&self, skill: Skill) -> Result<Skill, RepositoryError> {
        let updated = self.repository.update_skill(skill)?;
        self.count(true);
        Ok(updated)
    }

    fn soft_delete_skill(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
        let deleted = self.repository.soft_delete_skill(skill_id, deleted_at)?;
        self.count(deleted);
        Ok(deleted)
    }
}
