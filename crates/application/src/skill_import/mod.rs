//! Implements the two-phase batch skill import use cases.
//!
//! The module owns the `prepare -> preview -> commit` session lifecycle: preparation
//! materializes and validates a source snapshot without touching formal storage, preview
//! reports candidates and conflicts, and commit freezes decisions then runs a background
//! per-skill task that keeps database and formal-directory writes atomic per skill.

mod commit;
mod errors;
mod mapper;
mod ports;
mod service;

#[cfg(test)]
mod tests;

pub use errors::{DuplicateSkillName, SkillImportError};
pub use ports::{
    NoopSkillImportProgressPublisher, SkillImportConfig, SkillImportIdGenerator,
    SkillImportProgressEvent, SkillImportProgressPublisher, UuidSkillImportIdGenerator,
};
pub use service::SkillImportService;
