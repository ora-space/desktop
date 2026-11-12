//! Skill Effect planning and filesystem directory materialization.

use ora_effect::SkillName;

mod filesystem;
mod planner;

#[cfg(test)]
mod tests;

pub use filesystem::{
    MARKER_FILE_NAME, ManagedItemMarker, SkillDirectoryError, SkillDirectoryResourceAdapter,
};
pub use planner::SkillPlanner;

/// Returns the directory segment the Skill directory materializer uses for one Skill.
///
/// The native on-disk identity is the canonical [`SkillName`]: ASCII lowercase with every other
/// character, including `_`, preserved. An Agent-facing invocation name is a separate spelling and
/// may replace `_` with `-`; callers that freeze workspace-relative package paths must use this
/// identity so the path they report is the one Effect actually materializes.
pub fn materialized_skill_directory(name: &SkillName) -> &str {
    name.canonical()
}
