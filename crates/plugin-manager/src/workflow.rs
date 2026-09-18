use crate::limits::MAX_WORKFLOWS_PER_PACKAGE;
use crate::validation::{ManifestValidationError, invalid};
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use std::fs;
use std::path::Path;

/// Package-relative directory containing every workflow document a Workflow plugin contributes.
pub const WORKFLOW_ASSET_DIRECTORY: &str = "assets/workflows";

/// File extension every contributed workflow document must carry.
pub const WORKFLOW_FILE_EXTENSION: &str = "json";

/// Holds every workflow document contributed by one installed Workflow plugin.
///
/// The documents are not read here. Discovery proves only that the package carries a well-formed
/// workflow tree; the import use case parses each document when the user asks for it, which is
/// what lets one unparseable document be reported on its own instead of costing the user every
/// working workflow beside it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledWorkflowDescriptor {
    /// Package-relative paths of every contributed document, in stable file-name order.
    pub files: Vec<PortableRelativePath>,
}

/// Validates the workflow documents an installed Workflow plugin contributes.
///
/// A Workflow package is processless and carries no manifest section: everything it contributes
/// lives in its asset tree, so this check is the whole of what the host can verify before import.
pub(crate) fn validate_workflow(
    package_root: &Path,
) -> Result<InstalledWorkflowDescriptor, ManifestValidationError> {
    let directory = PortableRelativePath::parse(WORKFLOW_ASSET_DIRECTORY).map_err(|error| {
        invalid(
            "workflow",
            format!("Workflow asset directory name is invalid: {error}"),
        )
    })?;
    let package = CanonicalPathRoot::new(package_root).map_err(|error| {
        invalid(
            "workflow",
            format!("plugin package root is unavailable: {error}"),
        )
    })?;
    let asset_root = package.resolve_existing(&directory).map_err(|error| {
        invalid(
            "workflow",
            format!(
                "Workflow package must ship an `{WORKFLOW_ASSET_DIRECTORY}/` directory inside the package: {error}"
            ),
        )
    })?;
    if !asset_root.is_dir() {
        return Err(invalid(
            "workflow",
            format!("`{WORKFLOW_ASSET_DIRECTORY}` must be a directory"),
        ));
    }

    // Only regular `*.json` files directly under the asset directory are workflows. A README, a
    // subdirectory, or a symlink is ignored rather than rejected: the contract is "every document
    // in this directory is imported", and a stray file is no reason to refuse an otherwise valid
    // package. The entry type is read without following links, so a symlink can neither be
    // imported nor reach past the package root that `resolve_existing` already contained.
    let suffix = format!(".{WORKFLOW_FILE_EXTENSION}");
    let mut names = Vec::new();
    for entry in fs::read_dir(&asset_root).map_err(|error| {
        invalid(
            "workflow",
            format!("failed to read `{WORKFLOW_ASSET_DIRECTORY}/`: {error}"),
        )
    })? {
        let entry = entry.map_err(|error| {
            invalid(
                "workflow",
                format!("failed to enumerate `{WORKFLOW_ASSET_DIRECTORY}/`: {error}"),
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            invalid(
                "workflow",
                format!(
                    "failed to inspect `{WORKFLOW_ASSET_DIRECTORY}/{}`: {error}",
                    entry.file_name().to_string_lossy()
                ),
            )
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if file_type.is_file() && name.to_ascii_lowercase().ends_with(&suffix) {
            names.push(name);
        }
    }
    names.sort();

    if names.is_empty() {
        return Err(invalid(
            "workflow",
            format!(
                "`{WORKFLOW_ASSET_DIRECTORY}/` must contain at least one `{WORKFLOW_FILE_EXTENSION}` workflow document"
            ),
        ));
    }
    if names.len() > MAX_WORKFLOWS_PER_PACKAGE {
        return Err(invalid(
            "workflow",
            format!(
                "`{WORKFLOW_ASSET_DIRECTORY}/` must contain at most {MAX_WORKFLOWS_PER_PACKAGE} workflow documents, found {}",
                names.len()
            ),
        ));
    }

    let mut files = Vec::with_capacity(names.len());
    for name in names {
        let path = format!("{WORKFLOW_ASSET_DIRECTORY}/{name}");
        let relative = PortableRelativePath::parse(&path).map_err(|error| {
            invalid(
                "workflow",
                format!("workflow document `{name}` is not a portable package path: {error}"),
            )
        })?;
        files.push(relative);
    }

    Ok(InstalledWorkflowDescriptor { files })
}

#[cfg(test)]
mod tests {
    use super::{InstalledWorkflowDescriptor, WORKFLOW_ASSET_DIRECTORY, validate_workflow};
    use ora_utils::path::PortableRelativePath;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    /// Returns one expected descriptor for the given document file names.
    fn expected(names: &[&str]) -> InstalledWorkflowDescriptor {
        InstalledWorkflowDescriptor {
            files: names
                .iter()
                .map(|name| {
                    PortableRelativePath::parse(&format!("{WORKFLOW_ASSET_DIRECTORY}/{name}"))
                        .unwrap()
                })
                .collect(),
        }
    }

    #[test]
    fn accepts_one_or_more_workflow_documents() {
        let package = TempDir::new().unwrap();
        let asset_root = package.path().join(WORKFLOW_ASSET_DIRECTORY);
        fs::create_dir_all(&asset_root).unwrap();
        for name in ["2.0.0.json", "1.0.0.json"] {
            fs::write(asset_root.join(name), "{}").unwrap();
        }

        assert_eq!(
            validate_workflow(package.path()).unwrap(),
            expected(&["1.0.0.json", "2.0.0.json"])
        );
    }

    #[test]
    fn ignores_files_that_are_not_workflow_documents() {
        let package = TempDir::new().unwrap();
        let asset_root = package.path().join(WORKFLOW_ASSET_DIRECTORY);
        fs::create_dir_all(asset_root.join("nested")).unwrap();
        fs::write(asset_root.join("README.md"), "not a workflow").unwrap();
        fs::write(asset_root.join("nested/1.0.0.json"), "{}").unwrap();
        fs::write(asset_root.join("1.0.0.json"), "{}").unwrap();

        assert_eq!(
            validate_workflow(package.path()).unwrap(),
            expected(&["1.0.0.json"])
        );
    }

    #[test]
    fn rejects_missing_empty_and_documentless_asset_directories() {
        let missing = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        fs::create_dir_all(empty.path().join(WORKFLOW_ASSET_DIRECTORY)).unwrap();
        let documentless = TempDir::new().unwrap();
        let asset_root = documentless.path().join(WORKFLOW_ASSET_DIRECTORY);
        fs::create_dir_all(&asset_root).unwrap();
        fs::write(asset_root.join("README.md"), "not a workflow").unwrap();

        for package in [&missing, &empty, &documentless] {
            let error = validate_workflow(package.path()).unwrap_err();
            assert_eq!(error.field_path(), "workflow");
        }
    }

    #[test]
    fn rejects_more_documents_than_the_budget() {
        let package = TempDir::new().unwrap();
        let asset_root = package.path().join(WORKFLOW_ASSET_DIRECTORY);
        fs::create_dir_all(&asset_root).unwrap();
        for index in 0..=super::MAX_WORKFLOWS_PER_PACKAGE {
            fs::write(asset_root.join(format!("{index}.json")), "{}").unwrap();
        }

        let error = validate_workflow(package.path()).unwrap_err();
        assert_eq!(error.field_path(), "workflow");
    }
}
