//! Parsing of the `## Feature points` section of a module README.
//!
//! The section is the single source of feature point ids for the module and
//! doubles as the module's "should be covered" list, so its format is strict:
//! every top-level bullet must be `` - `id`: description ``.

use super::declaration::{TODO, is_feature_id};
use std::collections::BTreeMap;

/// Exact heading that opens the catalog section.
pub(crate) const SECTION_HEADING: &str = "## Feature points";

/// Parsed catalog of one README.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Catalog {
    /// Whether the README contains the section at all.
    pub(crate) has_section: bool,
    /// Feature id to its one-line description.
    pub(crate) entries: BTreeMap<String, String>,
    /// Format problems, each with the 1-based README line.
    pub(crate) problems: Vec<CatalogProblem>,
}

/// One malformed line inside the section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CatalogProblem {
    pub(crate) line: usize,
    pub(crate) message: String,
}

/// Parses a README's catalog section. Lines outside the section are ignored.
pub(crate) fn parse_catalog(markdown: &str) -> Catalog {
    let mut catalog = Catalog::default();
    let mut in_section = false;
    let mut in_fence = false;

    for (index, line) in markdown.lines().enumerate() {
        let line_number = index + 1;
        let trimmed_end = line.trim_end();

        // Fenced code blocks may contain anything, including fake bullets and headings.
        if trimmed_end.starts_with("```") || trimmed_end.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }

        if trimmed_end == SECTION_HEADING {
            in_section = true;
            catalog.has_section = true;
            continue;
        }
        if !in_section {
            continue;
        }
        // Any heading of level one or two closes the section; `###` is allowed for grouping.
        if trimmed_end.starts_with("# ") || trimmed_end.starts_with("## ") {
            in_section = false;
            continue;
        }
        let Some(item) = line.strip_prefix("- ") else {
            // Prose, blank lines, sub-headings, and indented continuation lines are not entries.
            continue;
        };
        match parse_entry(item) {
            Ok((id, description)) => {
                if id == TODO {
                    catalog.problems.push(CatalogProblem {
                        line: line_number,
                        message: format!(
                            "`{TODO}` is a reserved word and cannot be a feature point"
                        ),
                    });
                } else if catalog.entries.contains_key(id) {
                    catalog.problems.push(CatalogProblem {
                        line: line_number,
                        message: format!("duplicate feature point `{id}`"),
                    });
                } else {
                    catalog
                        .entries
                        .insert(id.to_string(), description.to_string());
                }
            }
            Err(message) => catalog.problems.push(CatalogProblem {
                line: line_number,
                message: message.to_string(),
            }),
        }
    }
    catalog
}

/// Parses `` `id`: description `` (the text after `- `).
fn parse_entry(item: &str) -> Result<(&str, &str), &'static str> {
    let rest = item
        .strip_prefix('`')
        .ok_or("feature point entries must start with a backticked id: - `id`: description")?;
    let (id, rest) = rest.split_once('`').ok_or("unterminated backticked id")?;
    if !is_feature_id(id) {
        return Err("feature id must be kebab-case: [a-z0-9]+(-[a-z0-9]+)*");
    }
    let rest = rest
        .strip_prefix(':')
        .ok_or("expected `:` directly after the backticked id")?;
    let description = rest.trim();
    if !rest.starts_with(' ') || description.is_empty() {
        return Err("expected a non-empty description after `: `");
    }
    Ok((id, description))
}
