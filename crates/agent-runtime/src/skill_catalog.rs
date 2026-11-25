//! Skill catalog prompt injection and ACP session setup metadata.
//!
//! The runtime owns when a live session is told the full catalog, when it receives only path
//! deltas, and how ACP `session/new` and `session/load` carry readable roots and setup metadata;
//! the host supplies the available entries and storage roots through [`crate::SkillCatalogSource`].

#[cfg(test)]
mod tests;

use crate::host::SkillCatalogEntry;
use agent_client_protocol_schema::v1::{
    ContentBlock, LoadSessionRequest, Meta, NewSessionRequest, TextContent,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::PathBuf;

/// ACP `_meta` key carrying the setup-time list of available Skill package directories.
pub const SKILL_DIRECTORIES_META_KEY: &str = "ora/skillDirectories";

/// In-memory record of the Skill entries already announced to one live provider session.
///
/// Keyed by each skill's `SKILL.md` absolute path because Agent reads by path: content updates at
/// the same path need no prompt notification, while renames or plugin version upgrades change the
/// path and must be surfaced as a removal plus addition. Names are retained alongside paths so a
/// later removal delta can name the skills that disappeared.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AnnouncedSkills {
    by_path: BTreeMap<PathBuf, String>,
}

impl AnnouncedSkills {
    /// Captures the path-to-name mapping of a catalog snapshot just sent to the provider.
    fn from_entries(entries: &[SkillCatalogEntry]) -> Self {
        let by_path = entries
            .iter()
            .map(|entry| (entry.skill_md_path.clone(), entry.name.clone()))
            .collect();
        Self { by_path }
    }
}

/// Prepared prompt block and the state to commit once the ACP `session/prompt` request is sent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SkillCatalogInjection {
    pub(crate) block_text: String,
    pub(crate) next_announced: AnnouncedSkills,
}

/// Computes the prompt block to inject for the current turn, or `None` when nothing needs telling.
///
/// When `announced` is `None`, the provider session has not been told yet (new actor, cold
/// `session/load`, rebuilt session, or post-compaction reset) and receives the full catalog if
/// non-empty. When `announced` is `Some`, only `SKILL.md` path additions and removals produce an
/// incremental update block; an identical path set emits nothing so prompt prefixes remain stable.
pub(crate) fn prepare_skill_catalog_injection(
    announced: Option<&AnnouncedSkills>,
    current: &[SkillCatalogEntry],
) -> Option<SkillCatalogInjection> {
    let sorted = sorted_distinct_entries(current);
    let next_announced = AnnouncedSkills::from_entries(&sorted);

    let Some(previous) = announced else {
        if sorted.is_empty() {
            return None;
        }
        return Some(SkillCatalogInjection {
            block_text: render_full_catalog(&sorted),
            next_announced,
        });
    };

    let current_paths: BTreeSet<&PathBuf> =
        sorted.iter().map(|entry| &entry.skill_md_path).collect();
    let added: Vec<&SkillCatalogEntry> = sorted
        .iter()
        .filter(|entry| !previous.by_path.contains_key(&entry.skill_md_path))
        .collect();
    let mut removed: Vec<(&PathBuf, &str)> = previous
        .by_path
        .iter()
        .filter(|(path, _)| !current_paths.contains(*path))
        .map(|(path, name)| (path, name.as_str()))
        .collect();
    removed.sort_by(|(path_a, name_a), (path_b, name_b)| {
        name_a
            .to_ascii_lowercase()
            .cmp(&name_b.to_ascii_lowercase())
            .then_with(|| path_a.cmp(path_b))
            .then_with(|| name_a.cmp(name_b))
    });

    if added.is_empty() && removed.is_empty() {
        return None;
    }

    Some(SkillCatalogInjection {
        block_text: render_delta_catalog(&added, &removed),
        next_announced,
    })
}

/// Inserts a catalog or delta block into the outgoing provider prompt.
///
/// The block precedes user content starting at `user_start_index` (after any transcript handoff
/// block), except when the first text block of user content begins with `/` (a user slash command
/// or workflow node invocation), in which case that leading command stays first so Agent CLIs
/// still parse it as a command.
pub(crate) fn insert_catalog_block(
    mut blocks: Vec<ContentBlock>,
    block_text: String,
    user_start_index: usize,
) -> Vec<ContentBlock> {
    let start = user_start_index.min(blocks.len());
    let first_text_offset = blocks[start..]
        .iter()
        .position(|block| matches!(block, ContentBlock::Text(_)));
    let insert_at = match first_text_offset {
        Some(offset) => match &blocks[start + offset] {
            ContentBlock::Text(text) if text.text.starts_with('/') => start + offset + 1,
            _ => start,
        },
        None => start,
    };
    blocks.insert(insert_at, ContentBlock::Text(TextContent::new(block_text)));
    blocks
}

/// Builds the `_meta` payload carrying current Skill package directories, or `None` when empty.
pub(crate) fn skill_directories_meta(entries: &[SkillCatalogEntry]) -> Option<Meta> {
    if entries.is_empty() {
        return None;
    }
    let sorted = sorted_distinct_entries(entries);
    let mut seen = BTreeSet::new();
    let mut directories = Vec::new();
    for entry in sorted {
        if seen.insert(entry.package_dir.clone()) {
            directories.push(serde_json::Value::String(
                entry.package_dir.to_string_lossy().into_owned(),
            ));
        }
    }
    if directories.is_empty() {
        return None;
    }
    let mut meta = Meta::new();
    meta.insert(
        SKILL_DIRECTORIES_META_KEY.to_string(),
        serde_json::Value::Array(directories),
    );
    Some(meta)
}

/// ACP session setup requests that can carry readable roots and skill directory metadata.
pub(crate) trait SkillSetupRequest: Sized {
    fn with_additional_directories(self, directories: Vec<PathBuf>) -> Self;
    fn with_meta(self, meta: Meta) -> Self;
}

impl SkillSetupRequest for NewSessionRequest {
    fn with_additional_directories(self, directories: Vec<PathBuf>) -> Self {
        self.additional_directories(directories)
    }

    fn with_meta(self, meta: Meta) -> Self {
        self.meta(meta)
    }
}

impl SkillSetupRequest for LoadSessionRequest {
    fn with_additional_directories(self, directories: Vec<PathBuf>) -> Self {
        self.additional_directories(directories)
    }

    fn with_meta(self, meta: Meta) -> Self {
        self.meta(meta)
    }
}

/// Attaches skill readable roots and `_meta` to an ACP `session/new` or `session/load` request
/// when applicable.
///
/// Hosts with empty catalogs and roots (such as `ora-node`) leave both fields untouched, keeping
/// the serialized ACP payload identical to prior releases. ACP specifies that `session/load`
/// carries the complete `additionalDirectories` list rather than a diff, so both setup requests
/// send the full list whenever the Agent advertised the capability.
pub(crate) fn apply_skill_setup<R: SkillSetupRequest>(
    mut request: R,
    additional_directories_supported: bool,
    extra_read_roots: Vec<PathBuf>,
    entries: &[SkillCatalogEntry],
) -> R {
    if additional_directories_supported && !extra_read_roots.is_empty() {
        request = request.with_additional_directories(extra_read_roots);
    }
    if let Some(meta) = skill_directories_meta(entries) {
        request = request.with_meta(meta);
    }
    request
}

/// Returns entries sorted deterministically by ASCII-lowercased name, then `SKILL.md` path.
fn sorted_distinct_entries(entries: &[SkillCatalogEntry]) -> Vec<SkillCatalogEntry> {
    let mut by_path = BTreeMap::new();
    for entry in entries {
        by_path
            .entry(entry.skill_md_path.clone())
            .or_insert_with(|| entry.clone());
    }
    let mut sorted: Vec<SkillCatalogEntry> = by_path.into_values().collect();
    sorted.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.skill_md_path.cmp(&b.skill_md_path))
            .then_with(|| a.name.cmp(&b.name))
    });
    sorted
}

/// Renders the initial full `<available_skills>` host context block.
fn render_full_catalog(entries: &[SkillCatalogEntry]) -> String {
    let mut text = String::from(
        "<available_skills>\n\
         Available skills are listed below. When a task matches a skill's description, read its SKILL.md first and follow its instructions. Relative paths inside SKILL.md resolve against its directory.\n",
    );
    for entry in entries {
        write_entry(&mut text, entry);
    }
    text.push_str("</available_skills>");
    text
}

/// Renders the incremental `<available_skills_update>` host context block.
fn render_delta_catalog(added: &[&SkillCatalogEntry], removed: &[(&PathBuf, &str)]) -> String {
    let mut text = String::from(
        "<available_skills_update>\n\
         Available skills changed during this session.\n",
    );
    if !added.is_empty() {
        text.push_str(
            "Added skills (read a skill's SKILL.md before using it; relative paths inside SKILL.md resolve against its directory):\n",
        );
        for entry in added {
            write_entry(&mut text, entry);
        }
    }
    if !removed.is_empty() {
        text.push_str("Removed skills (no longer available; do not use):\n");
        for (path, name) in removed {
            let _ = writeln!(text, "- {name}\n  {}", path.display());
        }
    }
    text.push_str("</available_skills_update>");
    text
}

/// Appends one skill entry formatted with its native `SKILL.md` path unescaped.
fn write_entry(text: &mut String, entry: &SkillCatalogEntry) {
    let description = normalize_inline_description(&entry.description);
    if description.is_empty() {
        let _ = writeln!(
            text,
            "- {}\n  {}",
            entry.name,
            entry.skill_md_path.display()
        );
    } else {
        let _ = writeln!(
            text,
            "- {}: {description}\n  {}",
            entry.name,
            entry.skill_md_path.display()
        );
    }
}

/// Collapses internal line breaks in descriptions so each skill entry stays compact and readable.
fn normalize_inline_description(description: &str) -> String {
    description.split_whitespace().collect::<Vec<_>>().join(" ")
}
