mod claims;
mod declaration;
mod journal;
mod ledger_validation;
mod mapping;
mod persistence;
mod projection_persistence;
mod queue;
mod recovery;
mod source;
mod store;
mod validation;

use crate::RepositoryPool;

/// SQLite adapter for the Generic Target Effect persistence interface.
#[derive(Clone, Debug)]
pub struct SqliteEffectRepository {
    pub(super) pool: RepositoryPool,
}

impl SqliteEffectRepository {
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
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
