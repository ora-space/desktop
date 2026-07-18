use gray_matter::Matter;
use gray_matter::engine::YAML;
use serde::Deserialize;
use std::path::{Component, Path, PathBuf};

/// Names the manifest file every uploaded skill folder must carry at its root.
pub(crate) const SKILL_MANIFEST_FILE: &str = "SKILL.md";

/// Carries the manifest fields parsed from the uploaded skill's `SKILL.md` front matter.
///
/// Unknown keys are ignored so richer skill manifests still import; only the two fields the skill
/// catalog persists are required here.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct SkillManifest {
    pub(crate) name: String,
    pub(crate) description: String,
}

/// Normalizes an uploaded file path into a safe in-package relative path, or `None` when unsafe.
///
/// Only `Normal` components are accepted so absolute paths, `..` traversal, `.` segments, and
/// drive prefixes can never escape the staging directory or collide with the `.tmp`/committed
/// directory layout the store depends on.
pub(crate) fn normalize_relative_path(raw: &str) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in Path::new(raw).components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::ParentDir => return None,
        }
    }

    if normalized.as_os_str().is_empty() {
        return None;
    }

    Some(normalized)
}

/// Reports whether a resolved skill name is a single safe directory segment.
///
/// The committed directory is named after the skill, so the name must be usable as one path
/// segment: restricting it to an ASCII slug alphabet and rejecting the `.`/`..` special segments
/// keeps the promote target inside `atoms/skills`.
pub(crate) fn is_safe_skill_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }

    name.chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
}

/// Parses the skill manifest front matter and returns its trimmed name and description.
///
/// Returns a human-readable reason string on any failure — absent front matter, malformed YAML, or
/// blank required fields — so the caller can surface it as one validation error.
pub(crate) fn parse_skill_manifest(contents: &str) -> Result<SkillManifest, String> {
    let matter = Matter::<YAML>::new();
    let parsed = matter
        .parse::<SkillManifest>(contents)
        .map_err(|error| format!("SKILL.md front matter is not valid YAML: {error}"))?;
    let manifest = parsed
        .data
        .ok_or_else(|| "SKILL.md must define name and description front matter".to_string())?;
    let name = manifest.name.trim().to_string();
    let description = manifest.description.trim().to_string();

    if name.is_empty() {
        return Err("SKILL.md front matter name must not be blank".to_string());
    }
    if description.is_empty() {
        return Err("SKILL.md front matter description must not be blank".to_string());
    }

    Ok(SkillManifest { name, description })
}

#[cfg(test)]
mod tests {
    use super::{is_safe_skill_name, normalize_relative_path, parse_skill_manifest};
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    #[test]
    fn normalizes_safe_nested_paths_and_rejects_traversal() {
        assert_eq!(
            normalize_relative_path("refs/util.py"),
            Some(PathBuf::from("refs").join("util.py"))
        );
        assert_eq!(
            normalize_relative_path("SKILL.md"),
            Some(PathBuf::from("SKILL.md"))
        );
        assert_eq!(normalize_relative_path(""), None);
        assert_eq!(normalize_relative_path("../escape"), None);
        assert_eq!(normalize_relative_path("/etc/passwd"), None);
        assert_eq!(normalize_relative_path("./here"), None);
    }

    #[test]
    fn accepts_slug_names_and_rejects_unsafe_ones() {
        assert_eq!(is_safe_skill_name("code-review"), true);
        assert_eq!(is_safe_skill_name("skill.v2_final"), true);
        assert_eq!(is_safe_skill_name("review / guide"), false);
        assert_eq!(is_safe_skill_name(".."), false);
        assert_eq!(is_safe_skill_name("."), false);
        assert_eq!(is_safe_skill_name(""), false);
    }

    #[test]
    fn parses_manifest_and_reports_blank_or_missing_fields() {
        let manifest =
            parse_skill_manifest("---\nname: grilling\ndescription: Grill the user\n---\nbody")
                .unwrap();
        assert_eq!(
            (manifest.name, manifest.description),
            ("grilling".to_string(), "Grill the user".to_string())
        );
        assert_eq!(
            parse_skill_manifest("no front matter here"),
            Err("SKILL.md must define name and description front matter".to_string())
        );
        assert_eq!(
            parse_skill_manifest("---\nname: grilling\ndescription: \"  \"\n---\n"),
            Err("SKILL.md front matter description must not be blank".to_string())
        );
    }
}
