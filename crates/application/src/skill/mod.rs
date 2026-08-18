mod filesystem_storage;
mod handlers;
mod id_generator;
mod mapper;
mod package_health;
mod ports;
mod storage;

#[cfg(test)]
mod tests;

pub use filesystem_storage::FilesystemSkillStorage;
pub use handlers::{
    CreateSkillHandler, DeleteSkillHandler, GetSkillHandler, ListSkillsHandler, UpdateSkillHandler,
};
pub use id_generator::UuidSkillIdGenerator;
pub use package_health::has_usable_package;
pub(crate) use package_health::{
    commit_existing_package, commit_restored_package, commit_unclaimed_package,
    persist_promoted_package,
};
pub use ports::{SkillIdGenerator, SkillRepository};
pub use storage::{
    BACKUP_DIR_NAME, CreateHandle, DeleteHandle, JOURNAL_DIR_NAME, JournalOp, JournalPhase,
    STAGING_DIR_NAME, SkillStorage, SkillStorageError, SwapHandle, TransactionJournal,
};
