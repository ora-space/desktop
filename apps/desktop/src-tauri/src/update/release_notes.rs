//! Reads the release notes that an update manifest may carry.
//!
//! The manifest is not covered by the package signature, so its notes are untrusted text. They are
//! read apart from the update check and can only ever be present or absent: a violation hides all of
//! them and never turns into an update error.

use chrono::NaiveDate;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

const MANIFEST_FIELD: &str = "ora_release_notes";
const SCHEMA: u64 = 1;
/// Limits on untrusted manifest notes. `scripts/release-notes.ts` writes within the same limits, which
/// may never exceed those of a released client, or that client would hide every note.
const MAX_VERSIONS: usize = 10;
const MAX_ENTRIES: usize = 50;
const MAX_TEXT_CHARS: usize = 500;

/// Names the kind of change one entry describes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseNoteCategory {
    Important,
    Feature,
    Improvement,
    Fix,
}

/// Carries one change in both interface languages.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReleaseNoteEntry {
    pub category: ReleaseNoteCategory,
    #[serde(rename = "zh-CN")]
    pub zh_cn: String,
    #[serde(rename = "en-US")]
    pub en_us: String,
}

/// Carries the notes of one stable version.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReleaseNote {
    pub version: String,
    pub date: String,
    pub entries: Vec<ReleaseNoteEntry>,
}

/// Explains why manifest notes stay hidden, for the Desktop log only.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub(super) enum HiddenNotes {
    #[error("the notes are malformed")]
    Malformed,
    #[error("the notes use an unknown schema")]
    UnknownSchema,
    #[error("the notes exceed the version limit")]
    TooManyVersions,
    #[error("a note breaks the content rules")]
    InvalidContent,
    #[error("the offered version is not a version")]
    InvalidTarget,
}

/// Returns the notes in `(stable(current), stable(target)]`, newest first.
///
/// Unknown fields are ignored so that a newer generator can add some. Any other deviation hides all
/// notes, because a list that silently lost some would pass for complete. Manifests without notes
/// have an empty list.
pub(super) fn notes_between(
    manifest: &Value,
    current: &Version,
    target: &str,
) -> Result<Vec<ReleaseNote>, HiddenNotes> {
    let Some(field) = manifest.get(MANIFEST_FIELD) else {
        return Ok(Vec::new());
    };
    let target = stable(&Version::parse(target).map_err(|_| HiddenNotes::InvalidTarget)?);
    let current = stable(current);
    // The schema decides how the rest is read, so it is checked before anything else.
    match field.get("schema").and_then(Value::as_u64) {
        Some(SCHEMA) => {}
        Some(_) => return Err(HiddenNotes::UnknownSchema),
        None => return Err(HiddenNotes::Malformed),
    }
    let listed = field
        .get("versions")
        .and_then(Value::as_array)
        .ok_or(HiddenNotes::Malformed)?;
    if listed.len() > MAX_VERSIONS {
        return Err(HiddenNotes::TooManyVersions);
    }
    let mut selected = Vec::new();
    for value in listed {
        let (version, note) = read_note(value).ok_or(HiddenNotes::InvalidContent)?;
        if version > current && version <= target {
            selected.push((version, note));
        }
    }
    selected.sort_by(|left, right| right.0.cmp(&left.0));
    Ok(selected.into_iter().map(|(_, note)| note).collect())
}

/// Reads one version's notes, enforcing the content rules of the files they were generated from.
fn read_note(value: &Value) -> Option<(Version, ReleaseNote)> {
    let note = ReleaseNote::deserialize(value).ok()?;
    let version = Version::parse(&note.version).ok()?;
    let usable = version == stable(&version)
        && is_calendar_date(&note.date)
        && (1..=MAX_ENTRIES).contains(&note.entries.len())
        && note
            .entries
            .iter()
            .flat_map(|entry| [&entry.zh_cn, &entry.en_us])
            .all(|text| is_plain_text(text) && text.chars().count() <= MAX_TEXT_CHARS);
    usable.then_some((version, note))
}

/// Accepts an existing `YYYY-MM-DD` date only.
fn is_calendar_date(text: &str) -> bool {
    text.len() == 10
        && text.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 4 | 7) {
                byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
        && text.parse::<NaiveDate>().is_ok()
}

/// Drops prerelease and build suffixes, because prereleases share the notes of their stable version.
fn stable(version: &Version) -> Version {
    Version::new(version.major, version.minor, version.patch)
}

/// Rejects blank text and characters that could hide or reorder what the text says.
fn is_plain_text(text: &str) -> bool {
    !text.trim().is_empty()
        && !text.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '\u{061C}'
                        | '\u{200E}'
                        | '\u{200F}'
                        | '\u{2028}'..='\u{202E}'
                        | '\u{2066}'..='\u{2069}'
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Builds one version's notes.
    fn note(version: &str) -> Value {
        json!({
            "version": version,
            "date": "2026-12-10",
            "entries": [{ "category": "fix", "zh-CN": "修复", "en-US": "Fixed" }],
        })
    }

    /// Builds a manifest that carries the given notes.
    fn manifest(notes: Vec<Value>) -> Value {
        json!({ "ora_release_notes": { "schema": 1, "versions": notes } })
    }

    /// Reads a manifest as an installation of `current` offered `target`.
    fn versions(manifest: &Value, current: &str, target: &str) -> Result<Vec<String>, HiddenNotes> {
        let current = Version::parse(current).expect("current version");
        Ok(notes_between(manifest, &current, target)?
            .into_iter()
            .map(|note| note.version)
            .collect())
    }

    /// Lists version text for comparison.
    fn listed(versions: &[&str]) -> Result<Vec<String>, HiddenNotes> {
        Ok(versions.iter().map(ToString::to_string).collect())
    }

    /// The range excludes the running and includes the offered stable version, newest first.
    #[test]
    fn selects_the_versions_an_update_would_bring() {
        let manifest = manifest(
            ["0.7.0", "0.10.0", "0.8.0", "0.9.0", "0.11.0"]
                .map(note)
                .to_vec(),
        );

        assert_eq!(
            versions(&manifest, "0.7.0", "0.10.0"),
            listed(&["0.10.0", "0.9.0", "0.8.0"])
        );
        assert_eq!(
            versions(&manifest, "0.7.0", "0.8.0-rc.1"),
            listed(&["0.8.0"])
        );
        assert_eq!(versions(&manifest, "0.7.0", "0.8.0+x"), listed(&["0.8.0"]));
        assert_eq!(versions(&manifest, "0.8.0-rc.1", "0.8.0"), listed(&[]));
        assert_eq!(versions(&manifest, "0.7.0", "0.7.0"), listed(&[]));
    }

    /// The wire form keeps the file's field names, which the webview reads.
    #[test]
    fn keeps_the_file_format_on_the_wire() {
        let notes = notes_between(
            &manifest(vec![note("0.8.0")]),
            &Version::new(0, 7, 0),
            "0.8.0",
        )
        .expect("notes are usable");

        assert_eq!(
            serde_json::to_value(notes).expect("notes serialize"),
            json!([note("0.8.0")])
        );
    }

    /// Both validators must agree, or a note that packaging accepts could hide every note.
    #[test]
    fn enforces_the_content_rules_of_the_notes_files() {
        let cases: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../../scripts/release-note-fixtures.json"
        ))
        .expect("shared fixtures parse");
        for case in cases {
            // A case that carries text may not even parse, which hides the notes as well.
            let note = match case["text"].as_str() {
                Some(text) => serde_json::from_str(text).unwrap_or(Value::Null),
                None => case["note"].clone(),
            };
            assert_eq!(
                (&case["name"], read_note(&note).is_some()),
                (&case["name"], case["valid"].as_bool().expect("valid flag")),
            );
        }
    }

    /// One unusable note hides every note, including the valid ones beside it.
    #[test]
    fn hides_all_notes_when_any_of_them_is_unusable() {
        let mut broken = note("0.9.0");
        broken["entries"] = json!([]);
        assert_eq!(
            versions(&manifest(vec![note("0.8.0"), broken]), "0.7.0", "0.8.0"),
            Err(HiddenNotes::InvalidContent)
        );
        assert_eq!(
            versions(
                &manifest(
                    (0..=MAX_VERSIONS)
                        .map(|minor| note(&format!("0.{minor}.0")))
                        .collect()
                ),
                "0.7.0",
                "0.8.0"
            ),
            Err(HiddenNotes::TooManyVersions)
        );
    }

    /// A later generator may add fields, but a schema this build does not know hides the notes.
    #[test]
    fn ignores_unknown_fields_and_hides_unknown_schemas() {
        let mut extended = note("0.8.0");
        extended["prs"] = json!([1]);
        extended["entries"][0]["link"] = json!("https://example.invalid");
        let mut manifest = manifest(vec![extended]);
        manifest["ora_release_notes"]["generator"] = json!("later");
        assert_eq!(versions(&manifest, "0.7.0", "0.8.0"), listed(&["0.8.0"]));

        manifest["ora_release_notes"]["schema"] = json!(2);
        assert_eq!(
            versions(&manifest, "0.7.0", "0.8.0"),
            Err(HiddenNotes::UnknownSchema)
        );
    }

    /// Manifests without notes are ordinary; another shape in their place is only hidden.
    #[test]
    fn accepts_manifests_without_notes() {
        assert_eq!(versions(&json!({}), "0.7.0", "0.8.0"), listed(&[]));
        assert_eq!(versions(&json!([]), "0.7.0", "0.8.0"), listed(&[]));
        for field in [
            json!(null),
            json!("text"),
            json!([]),
            json!({ "schema": 1 }),
        ] {
            assert_eq!(
                versions(&json!({ "ora_release_notes": field }), "0.7.0", "0.8.0"),
                Err(HiddenNotes::Malformed)
            );
        }
        assert_eq!(
            versions(&manifest(vec![note("0.8.0")]), "0.7.0", "nightly"),
            Err(HiddenNotes::InvalidTarget)
        );
    }
}
