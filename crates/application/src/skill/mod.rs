mod filesystem_storage;
mod handlers;
mod id_generator;
mod mapper;
mod ports;
mod storage;

#[cfg(test)]
mod tests;

pub use filesystem_storage::FilesystemSkillStorage;
pub use handlers::{
    CreateSkillHandler, DeleteSkillHandler, GetSkillHandler, ListSkillsHandler, UpdateSkillHandler,
};
pub use id_generator::UuidSkillIdGenerator;
pub use ports::{SkillIdGenerator, SkillRepository};
pub use storage::{
    BACKUP_DIR_NAME, CreateHandle, DeleteHandle, JOURNAL_DIR_NAME, JournalOp, JournalPhase,
    STAGING_DIR_NAME, SkillStorage, SkillStorageError, SwapHandle, TransactionJournal,
};
