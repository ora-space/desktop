mod claims;
mod conditions;
mod declaration;
mod journal;
mod ledger_validation;
mod mapping;
mod operations;
mod persistence;
mod projection_persistence;
mod queue;
mod read;
mod recovery;
mod source;
mod store;
mod validation;
mod write;

use crate::{LocalTimestampSource, RepositoryPool, TimestampSource};
pub(crate) use write::EffectWriteContext;

/// SQLite adapter for the Generic Target Effect persistence interface.
#[derive(Clone, Debug)]
pub struct SqliteEffectRepository<Clock = LocalTimestampSource> {
    pub(super) pool: RepositoryPool,
    pub(super) clock: Clock,
}

impl SqliteEffectRepository {
    pub fn new(pool: RepositoryPool) -> Self {
        Self {
            pool,
            clock: LocalTimestampSource,
        }
    }
}

impl<Clock: TimestampSource> SqliteEffectRepository<Clock> {
    /// Supplies an independent clock without letting callers choose individual audit timestamps.
    pub fn with_clock(pool: RepositoryPool, clock: Clock) -> Self {
        Self { pool, clock }
    }
}

pub(crate) use source::{
    PublishedSkillRevision, advance_changed_scopes, publish_skill_revision, retire_skill_source,
    seed_scope_sources,
};

/// Result of explicitly retiring one stable Effect source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceMutationOutcome {
    Retired,
    Missing,
}
