use crate::ApplicationError;
use crate::skill::ports::{SkillPackageStore, SkillPackageStoreError, SkillRepository};
use std::collections::HashSet;

/// Reconciles the `atoms/skills` directory against the skill catalog during startup.
///
/// A hard crash can leave two kinds of debris that in-process rollback never got to clean: an
/// incomplete `<id>.tmp` staging directory, or a `<name>` directory that was promoted just before
/// its commit landed. This handler discards every staging directory and removes any committed
/// directory that no visible skill row claims, restoring the "roll back leaves the folder clean"
/// guarantee after a crash.
pub struct ReconcileSkillStorageHandler<Store, Repository> {
    store: Store,
    repository: Repository,
}

impl<Store, Repository> ReconcileSkillStorageHandler<Store, Repository> {
    /// Builds the reconciliation handler from its filesystem and catalog ports.
    pub fn new(store: Store, repository: Repository) -> Self {
        Self { store, repository }
    }
}

impl<Store, Repository> ReconcileSkillStorageHandler<Store, Repository>
where
    Store: SkillPackageStore,
    Repository: SkillRepository,
{
    /// Sweeps staging debris and orphaned committed directories not backed by a visible skill.
    pub fn handle(&self) -> Result<(), ApplicationError> {
        self.store.remove_all_staging().map_err(storage_error)?;

        let visible_names: HashSet<String> = self
            .repository
            .list_skills()
            .map_err(ApplicationError::from_skill_repository_error)?
            .into_iter()
            .map(|skill| skill.name)
            .collect();

        for name in self.store.list_committed_names().map_err(storage_error)? {
            if !visible_names.contains(&name) {
                self.store.remove_committed(&name).map_err(storage_error)?;
            }
        }

        Ok(())
    }
}

/// Maps skill filesystem failures onto the stable storage application error.
fn storage_error(error: SkillPackageStoreError) -> ApplicationError {
    match error {
        SkillPackageStoreError::OperationFailed(message) => {
            ApplicationError::SkillPackageStorage { message }
        }
    }
}
