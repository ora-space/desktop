use crate::validation::{ManifestValidationError, invalid};
use ora_skill_package::Limits;
use ora_skill_package::manifest::parse_manifest;
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Package-relative directory containing every Skill contributed by a Skill plugin.
pub const SKILL_ASSET_DIRECTORY: &str = "assets/skills";
/// Fixed manifest filename required at the root of each contributed Skill.
pub const SKILL_MANIFEST_FILE_NAME: &str = "SKILL.md";

/// Holds every validated Skill contributed by one installed plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledSkillDescriptor {
    pub skills: Vec<InstalledSkill>,
}

/// Holds the catalog metadata and immutable package root of one contributed Skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledSkill {
    pub name: String,
    pub description: String,
    pub package_root: PathBuf,
}

/// Validates and parses every Skill shipped by an installed Skill plugin.
pub(crate) fn validate_skill(
    package_root: &Path,
) -> Result<InstalledSkillDescriptor, ManifestValidationError> {
    let skills_relative = PortableRelativePath::parse(SKILL_ASSET_DIRECTORY).map_err(|error| {
        invalid(
            "skill",
            format!("Skill asset directory name is invalid: {error}"),
        )
    })?;
    let package = CanonicalPathRoot::new(package_root).map_err(|error| {
        invalid(
            "skill",
            format!("plugin package root is unavailable: {error}"),
        )
    })?;
    let skills_root = package.resolve_existing(&skills_relative).map_err(|error| {
        invalid(
            "skill",
            format!(
                "Skill package must ship an `{SKILL_ASSET_DIRECTORY}/` directory inside the package: {error}"
            ),
        )
    })?;
    if !skills_root.is_dir() {
        return Err(invalid(
            "skill",
            format!("`{SKILL_ASSET_DIRECTORY}` must be a directory"),
        ));
    }

    let mut skill_roots = fs::read_dir(&skills_root)
        .map_err(|error| {
            invalid(
                "skill",
                format!("failed to read `{SKILL_ASSET_DIRECTORY}/`: {error}"),
            )
        })?
        .filter_map(|entry| match entry {
            Ok(entry) => match entry.file_type() {
                Ok(file_type) if file_type.is_dir() => Some(Ok(entry.path())),
                Ok(_) => None,
                Err(error) => Some(Err(invalid(
                    "skill",
                    format!(
                        "failed to inspect `{SKILL_ASSET_DIRECTORY}/{}/`: {error}",
                        entry.file_name().to_string_lossy()
                    ),
                ))),
            },
            Err(error) => Some(Err(invalid(
                "skill",
                format!("failed to enumerate `{SKILL_ASSET_DIRECTORY}/`: {error}"),
            ))),
        })
        .collect::<Result<Vec<PathBuf>, ManifestValidationError>>()?;
    skill_roots.sort();

    if skill_roots.is_empty() {
        return Err(invalid(
            "skill",
            format!("`{SKILL_ASSET_DIRECTORY}/` must contain at least one Skill directory"),
        ));
    }

    let manifest_relative =
        PortableRelativePath::parse(SKILL_MANIFEST_FILE_NAME).map_err(|error| {
            invalid(
                "skill",
                format!("Skill manifest filename is invalid: {error}"),
            )
        })?;
    let mut installed = Vec::with_capacity(skill_roots.len());
    let mut names = BTreeSet::new();
    for skill_root in skill_roots {
        let skill_name = skill_root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<unknown>".to_owned());
        let skill = CanonicalPathRoot::new(&skill_root).map_err(|error| {
            invalid(
                "skill",
                format!(
                    "Skill directory `{SKILL_ASSET_DIRECTORY}/{skill_name}/` is unavailable: {error}"
                ),
            )
        })?;
        let manifest = skill.resolve_existing(&manifest_relative).map_err(|error| {
            invalid(
                "skill",
                format!(
                    "Skill directory `{SKILL_ASSET_DIRECTORY}/{skill_name}/` must ship `{SKILL_MANIFEST_FILE_NAME}`: {error}"
                ),
            )
        })?;
        if !manifest.is_file() {
            return Err(invalid(
                "skill",
                format!(
                    "`{SKILL_ASSET_DIRECTORY}/{skill_name}/{SKILL_MANIFEST_FILE_NAME}` must be a regular file"
                ),
            ));
        }
        let bytes = fs::read(&manifest).map_err(|error| {
            invalid(
                "skill",
                format!(
                    "failed to read `{SKILL_ASSET_DIRECTORY}/{skill_name}/{SKILL_MANIFEST_FILE_NAME}`: {error}"
                ),
            )
        })?;
        let parsed = parse_manifest(&bytes, Limits::default().max_manifest_bytes).map_err(|error| {
            invalid(
                "skill",
                format!(
                    "`{SKILL_ASSET_DIRECTORY}/{skill_name}/{SKILL_MANIFEST_FILE_NAME}` is invalid: {error}"
                ),
            )
        })?;
        if !parsed.name.eq_ignore_ascii_case(&skill_name) {
            return Err(invalid(
                "skill",
                format!(
                    "Skill directory `{skill_name}` must match the SKILL.md name `{}`",
                    parsed.name
                ),
            ));
        }
        if !names.insert(parsed.name.to_ascii_lowercase()) {
            return Err(invalid(
                "skill",
                format!("duplicate Skill name `{}`", parsed.name),
            ));
        }
        installed.push(InstalledSkill {
            name: parsed.name,
            description: parsed.description,
            package_root: skill_root,
        });
    }

    Ok(InstalledSkillDescriptor { skills: installed })
}

#[cfg(test)]
mod tests {
    use super::{SKILL_ASSET_DIRECTORY, SKILL_MANIFEST_FILE_NAME, validate_skill};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn accepts_one_or_more_skill_directories() {
        let package = TempDir::new().unwrap();
        for name in ["review", "testing"] {
            let root = package.path().join(SKILL_ASSET_DIRECTORY).join(name);
            fs::create_dir_all(root.join("scripts")).unwrap();
            fs::write(
                root.join(SKILL_MANIFEST_FILE_NAME),
                format!("---\nname: {name}\ndescription: Test skill\n---\n"),
            )
            .unwrap();
        }

        assert!(validate_skill(package.path()).is_ok());
    }

    #[test]
    fn rejects_missing_empty_and_incomplete_skill_trees() {
        let missing = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        fs::create_dir_all(empty.path().join(SKILL_ASSET_DIRECTORY)).unwrap();
        let incomplete = TempDir::new().unwrap();
        fs::create_dir_all(incomplete.path().join(SKILL_ASSET_DIRECTORY).join("review")).unwrap();

        for package in [&missing, &empty, &incomplete] {
            let error = validate_skill(package.path()).unwrap_err();
            assert_eq!(error.field_path(), "skill");
        }
    }
}
