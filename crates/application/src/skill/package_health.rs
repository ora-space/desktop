use super::storage::{CreateHandle, JournalOp, SkillStorage, SwapHandle};
use crate::ApplicationError;
use ora_contracts::SkillAvailability;
use ora_domain::Namespace;
use ora_skill_package::Limits;
use ora_skill_package::manifest::parse_manifest;
use std::path::Path;

/// Handle for a newly promoted formal package that may have replaced an untracked leftover.
pub(crate) enum PromotedPackage {
    Created(CreateHandle),
    Replaced(SwapHandle),
}

impl PromotedPackage {
    /// Restores the previous formal directory when the database write did not commit.
    pub(crate) fn rollback<Storage: SkillStorage>(&self, storage: &Storage) {
        let result = match self {
            Self::Created(handle) => storage.rollback_create(handle),
            Self::Replaced(handle) => storage.rollback_swap(handle),
        };
        if let Err(error) = result {
            ora_logging::ora_warn!(
                error = %error,
                "failed to roll back skill package transaction; startup reconciliation will retry"
            );
        }
    }

    /// Removes journal and compensation artifacts after the database write succeeds.
    pub(crate) fn finish<Storage: SkillStorage>(
        &self,
        storage: &Storage,
    ) -> Result<(), ApplicationError> {
        match self {
            Self::Created(handle) => storage.finish_create(handle),
            Self::Replaced(handle) => storage.finish_swap(handle),
        }
        .map_err(ApplicationError::from_skill_storage_error)
    }
}

/// Returns whether `SKILL.md` bytes parse as a skill manifest.
pub(crate) fn manifest_is_usable(bytes: &[u8]) -> bool {
    parse_manifest(bytes, Limits::default().max_manifest_bytes).is_ok()
}

/// Returns whether the formal directory has a root `SKILL.md` that parses as a skill manifest.
pub fn has_usable_package<Storage: SkillStorage>(
    storage: &Storage,
    name: &str,
) -> Result<bool, ApplicationError> {
    Ok(storage
        .read_manifest(name)
        .map_err(ApplicationError::from_skill_storage_error)?
        .is_some_and(|bytes| manifest_is_usable(&bytes)))
}

/// Derives catalog availability from the on-disk package. Missing or unreadable files do
/// not forget the row; the skill stays visible as unavailable until the user deletes it
/// or restores a package with the same name.
pub(crate) fn package_availability<Storage: SkillStorage>(
    storage: &Storage,
    name: &str,
) -> Result<SkillAvailability, ApplicationError> {
    if has_usable_package(storage, name)? {
        Ok(SkillAvailability::Available)
    } else {
        Ok(SkillAvailability::Unavailable)
    }
}

/// Frees a catalog name after a successful delete when `commit_delete` already saw no directory.
///
/// Create and import of an unclaimed name must call [`commit_unclaimed_package`] instead so a
/// leftover complete package is journaled and can be restored.
pub(crate) fn claim_untracked_name<Storage: SkillStorage>(
    storage: &Storage,
    name: &str,
) -> Result<(), ApplicationError> {
    storage
        .remove_formal(name)
        .map_err(ApplicationError::from_skill_storage_error)
}

/// Promotes staging into `<name>`, replacing an existing leftover through a journaled swap.
///
/// `commit_create` cannot succeed while the formal directory exists, so claiming an untracked
/// name must not delete first. A swap keeps the leftover in the compensation backup until the
/// database write commits.
pub(crate) fn commit_unclaimed_package<Storage: SkillStorage>(
    storage: &Storage,
    name: &str,
    staging: &Path,
) -> Result<PromotedPackage, ApplicationError> {
    if storage.formal_exists(name)
        && storage
            .list_journals()
            .map_err(ApplicationError::from_skill_storage_error)?
            .iter()
            .any(|journal| {
                matches!(journal.op, JournalOp::Create | JournalOp::Swap)
                    && (journal.name == name || journal.from_name == name)
            })
    {
        return Err(ApplicationError::SkillFolderConflict {
            name: name.to_string(),
        });
    }
    promote_staging(storage, name, staging)
}

/// Promotes staging for an unavailable catalog row, optionally renaming its old package.
///
/// Restore must not use [`commit_unclaimed_package`]: that helper claims even a complete leftover.
/// A rename never overwrites another target directory, usable or otherwise. When the previous
/// directory exists it is moved into the compensation backup by `commit_swap`, preserving the
/// whole residual package for rollback. A missing previous directory uses `commit_create`.
pub(crate) fn commit_restored_package<Storage: SkillStorage>(
    storage: &Storage,
    namespace: &Namespace,
    name: &str,
    previous_name: &str,
    staging: &Path,
) -> Result<PromotedPackage, ApplicationError> {
    if name == previous_name && has_usable_package(storage, name)? {
        return Err(ApplicationError::SkillNameConflict {
            namespace: namespace.to_string(),
            name: name.to_string(),
        });
    }
    if name != previous_name && storage.formal_exists(name) {
        return Err(ApplicationError::SkillNameConflict {
            namespace: namespace.to_string(),
            name: name.to_string(),
        });
    }

    promote_from(storage, name, previous_name, staging).map_err(|error| match error {
        super::storage::SkillStorageError::FormalDirectoryExists { .. } => {
            ApplicationError::SkillNameConflict {
                namespace: namespace.to_string(),
                name: name.to_string(),
            }
        }
        error => ApplicationError::from_skill_storage_error(error),
    })
}

/// Promotes staging over an existing package when import overwrite has already revalidated it.
pub(crate) fn commit_existing_package<Storage: SkillStorage>(
    storage: &Storage,
    name: &str,
    previous_name: &str,
    staging: &Path,
) -> Result<PromotedPackage, ApplicationError> {
    promote_from(storage, name, previous_name, staging)
        .map_err(ApplicationError::from_skill_storage_error)
}

/// Couples a promoted package with its repository write.
///
/// A failed repository write restores the old package. Once the repository write succeeds the
/// transaction is committed; journal/backup cleanup is deliberately best-effort because startup
/// reconciliation can reclaim those artifacts and callers must not see a committed mutation as a
/// failure merely because post-commit cleanup failed.
pub(crate) fn persist_promoted_package<Storage, T, E>(
    storage: &Storage,
    promoted: &PromotedPackage,
    persist: impl FnOnce() -> Result<T, E>,
) -> Result<T, E>
where
    Storage: SkillStorage,
{
    match persist() {
        Ok(value) => {
            if let Err(error) = promoted.finish(storage) {
                ora_logging::ora_warn!(
                    error = %error,
                    "failed to finish committed skill package transaction; startup reconciliation will clean it"
                );
            }
            Ok(value)
        }
        Err(error) => {
            promoted.rollback(storage);
            Err(error)
        }
    }
}

/// Promotes staging into `<name>`, swapping when a leftover directory is already present.
fn promote_staging<Storage: SkillStorage>(
    storage: &Storage,
    name: &str,
    staging: &Path,
) -> Result<PromotedPackage, ApplicationError> {
    if storage.formal_exists(name) {
        storage
            .commit_swap(name, name, staging)
            .map(PromotedPackage::Replaced)
            .map_err(ApplicationError::from_skill_storage_error)
    } else {
        storage
            .commit_create(name, staging)
            .map(PromotedPackage::Created)
            .map_err(ApplicationError::from_skill_storage_error)
    }
}

/// Promotes staging from `previous_name`, using a journaled rename when that directory exists.
fn promote_from<Storage: SkillStorage>(
    storage: &Storage,
    name: &str,
    previous_name: &str,
    staging: &Path,
) -> Result<PromotedPackage, super::storage::SkillStorageError> {
    if storage.formal_exists(previous_name) {
        storage
            .commit_swap(name, previous_name, staging)
            .map(PromotedPackage::Replaced)
    } else {
        storage
            .commit_create(name, staging)
            .map(PromotedPackage::Created)
    }
}
