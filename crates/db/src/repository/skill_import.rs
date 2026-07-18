use ora_application::{SkillImportCommitError, SkillImportUnitOfWork};
use ora_domain::Skill;

use super::RepositoryPool;
use super::skill::insert_skill_row;

/// Commits a skill import as one SQLite transaction spanning the row insert and directory promote.
///
/// The transaction stays open across the caller-provided promote so the row only becomes durable
/// once the directory rename has succeeded; any promote error rolls the insert back untouched.
#[derive(Clone, Debug)]
pub struct SqliteSkillImportUnitOfWork {
    pool: RepositoryPool,
}

impl SqliteSkillImportUnitOfWork {
    /// Builds the import unit of work from the shared SQLite connection pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl SkillImportUnitOfWork for SqliteSkillImportUnitOfWork {
    fn insert_then<OnInserted>(
        &self,
        skill: Skill,
        on_inserted: OnInserted,
    ) -> Result<(), SkillImportCommitError>
    where
        OnInserted: FnOnce() -> Result<(), String>,
    {
        // The inner `Result` carries the import outcome; the outer `DatabaseError` only covers pool
        // checkout and `BEGIN`, both of which precede any promote and are therefore insert-phase.
        let outcome = self.pool.with_connection_mut(|connection| {
            let transaction = connection.transaction()?;
            if let Err(error) = insert_skill_row(&transaction, &skill) {
                return Ok(Err(SkillImportCommitError::Insert {
                    message: error.to_string(),
                }));
            }

            match on_inserted() {
                Ok(()) => match transaction.commit() {
                    Ok(()) => Ok(Ok(())),
                    Err(error) => Ok(Err(SkillImportCommitError::CommitAfterPromote {
                        message: error.to_string(),
                    })),
                },
                Err(message) => {
                    // Dropping the transaction rolls the insert back so no row is ever committed.
                    drop(transaction);
                    Ok(Err(SkillImportCommitError::Promote { message }))
                }
            }
        });

        match outcome {
            Ok(inner) => inner,
            Err(error) => Err(SkillImportCommitError::Insert {
                message: error.to_string(),
            }),
        }
    }
}
