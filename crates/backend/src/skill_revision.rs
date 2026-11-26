//! Tells which discoveries were made before the Skills last changed.

use ora_application::{
    CreateHandle, DeleteHandle, RepositoryError, SkillRepository, SkillStorage, SkillStorageError,
    SwapHandle, TransactionJournal,
};
use ora_db::{DatabaseError, PluginSkillProjection, SqliteSkillRepository};
use ora_domain::{Namespace, PluginId, Skill, SkillId};
use ora_utils::path::StrictRelativePath;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
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

    /// Advances after a change whose package files already reached the disk was rolled back.
    ///
    /// Nothing was committed, but whoever looked at the Skills meanwhile may hold a package that
    /// never became part of the catalog, or a catalog missing one that is back now.
    pub(crate) fn rolled_back(&self) {
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

/// The Skill packages as the code that changes them sees them: a change rolled back after its
/// files reached the disk is counted.
///
/// The catalog is counted on commit (see [`CountedSkillRepository`]), but an edit swaps its new
/// package in, and a delete or rename moves the package aside, before the row is written. A failed
/// write then puts the old package back. Discovery in between may have cached the swapped-in
/// package, or a catalog without the moved one, under the revision that is current again after the
/// rollback, so the rollback advances it once. A change that fails before touching the disk, or
/// whose rollback only removes what nobody could discover yet (a create), is not counted.
#[derive(Clone)]
pub(crate) struct CountedSkillStorage<S> {
    storage: S,
    revision: SkillRevision,
}

impl<S> CountedSkillStorage<S> {
    pub(crate) fn new(storage: S, revision: SkillRevision) -> Self {
        Self { storage, revision }
    }
}

impl<S: SkillStorage> SkillStorage for CountedSkillStorage<S> {
    fn formal_package_path(&self, name: &str) -> Option<PathBuf> {
        self.storage.formal_package_path(name)
    }

    fn create_staging(&self) -> Result<PathBuf, SkillStorageError> {
        self.storage.create_staging()
    }

    fn stage_existing(&self, name: &str, staging: &Path) -> Result<(), SkillStorageError> {
        self.storage.stage_existing(name, staging)
    }

    fn write_file(
        &self,
        staging: &Path,
        relative: &StrictRelativePath,
        bytes: &[u8],
    ) -> Result<(), SkillStorageError> {
        self.storage.write_file(staging, relative, bytes)
    }

    fn copy_file(
        &self,
        staging: &Path,
        relative: &StrictRelativePath,
        source: &Path,
    ) -> Result<(), SkillStorageError> {
        self.storage.copy_file(staging, relative, source)
    }

    fn write_manifest(&self, staging: &Path, content: &[u8]) -> Result<(), SkillStorageError> {
        self.storage.write_manifest(staging, content)
    }

    fn commit_create(
        &self,
        name: &str,
        skill_id: &SkillId,
        staging: &Path,
    ) -> Result<CreateHandle, SkillStorageError> {
        self.storage.commit_create(name, skill_id, staging)
    }

    fn rollback_create(&self, handle: &CreateHandle) -> Result<(), SkillStorageError> {
        self.storage.rollback_create(handle)
    }

    fn finish_create(&self, handle: &CreateHandle) -> Result<(), SkillStorageError> {
        self.storage.finish_create(handle)
    }

    fn commit_swap(
        &self,
        name: &str,
        from_name: &str,
        skill_id: &SkillId,
        previous_updated_at: Option<i64>,
        staging: &Path,
    ) -> Result<SwapHandle, SkillStorageError> {
        self.storage
            .commit_swap(name, from_name, skill_id, previous_updated_at, staging)
    }

    /// Counts once the old package is back, or failed to come back: the swapped-in one was on disk
    /// either way.
    fn rollback_swap(&self, handle: &SwapHandle) -> Result<(), SkillStorageError> {
        let result = self.storage.rollback_swap(handle);
        self.revision.rolled_back();
        result
    }

    fn finish_swap(&self, handle: &SwapHandle) -> Result<(), SkillStorageError> {
        self.storage.finish_swap(handle)
    }

    fn commit_delete(
        &self,
        name: &str,
        skill_id: &SkillId,
    ) -> Result<DeleteHandle, SkillStorageError> {
        self.storage.commit_delete(name, skill_id)
    }

    /// Counts once the package is back, or failed to come back: it was moved aside either way.
    fn rollback_delete(&self, handle: &DeleteHandle) -> Result<(), SkillStorageError> {
        let result = self.storage.rollback_delete(handle);
        self.revision.rolled_back();
        result
    }

    fn finish_delete(&self, handle: &DeleteHandle) -> Result<(), SkillStorageError> {
        self.storage.finish_delete(handle)
    }

    fn formal_exists(&self, name: &str) -> bool {
        self.storage.formal_exists(name)
    }

    fn read_package_manifest(
        &self,
        package_root: &Path,
    ) -> Result<Option<Vec<u8>>, SkillStorageError> {
        self.storage.read_package_manifest(package_root)
    }

    fn read_manifest(&self, name: &str) -> Result<Option<Vec<u8>>, SkillStorageError> {
        self.storage.read_manifest(name)
    }

    fn list_formal_names(&self) -> Result<Vec<String>, SkillStorageError> {
        self.storage.list_formal_names()
    }

    fn remove_temp(&self, path: &Path) -> Result<(), SkillStorageError> {
        self.storage.remove_temp(path)
    }

    fn restore_backup(&self, backup: &Path, name: &str) -> Result<(), SkillStorageError> {
        self.storage.restore_backup(backup, name)
    }

    fn remove_dir(&self, path: &Path) -> Result<(), SkillStorageError> {
        self.storage.remove_dir(path)
    }

    fn remove_formal(&self, name: &str) -> Result<(), SkillStorageError> {
        self.storage.remove_formal(name)
    }

    fn list_journals(&self) -> Result<Vec<TransactionJournal>, SkillStorageError> {
        self.storage.list_journals()
    }

    fn remove_journal(&self, journal: &TransactionJournal) -> Result<(), SkillStorageError> {
        self.storage.remove_journal(journal)
    }
}
