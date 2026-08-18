//! Filesystem resolution: which files are in scope and which README owns a test file.

use std::fs;
use std::path::{Path, PathBuf};

/// Directory names that are never scanned.
const SKIPPED_DIRECTORIES: &[&str] = &["target", "node_modules", ".git"];

/// Files discovered under the requested scope, split by role.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ScopedFiles {
    pub(crate) rust_files: Vec<PathBuf>,
    pub(crate) readmes: Vec<PathBuf>,
}

/// Collects `.rs` files and `README.md` files under each scope path (file or directory).
pub(crate) fn collect_scope(paths: &[PathBuf]) -> Result<ScopedFiles, String> {
    let mut files = ScopedFiles::default();
    for path in paths {
        if path.is_file() {
            classify(path, &mut files);
        } else if path.is_dir() {
            walk(path, &mut files)?;
        } else {
            return Err(format!("scope path does not exist: {}", path.display()));
        }
    }
    files.rust_files.sort();
    files.readmes.sort();
    Ok(files)
}

/// Recursively visits a directory, skipping build and dependency folders.
fn walk(directory: &Path, files: &mut ScopedFiles) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|error| format!("failed to read entry in {}: {error}", directory.display()))?
            .path();
        if path.is_dir() {
            let skip = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| SKIPPED_DIRECTORIES.contains(&name));
            if !skip {
                walk(&path, files)?;
            }
        } else {
            classify(&path, files);
        }
    }
    Ok(())
}

/// Buckets a file by role; anything else is ignored.
fn classify(path: &Path, files: &mut ScopedFiles) {
    if path.extension().is_some_and(|extension| extension == "rs") {
        files.rust_files.push(path.to_path_buf());
    } else if path.file_name().is_some_and(|name| name == "README.md") {
        files.readmes.push(path.to_path_buf());
    }
}

/// Where a test file's declarations resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ownership {
    /// Directory containing the crate's `Cargo.toml`.
    pub(crate) crate_root: PathBuf,
    /// Nearest `README.md` from the file's directory up to the crate root, if any.
    pub(crate) owning_readme: Option<PathBuf>,
}

/// Resolves the crate root and owning README for a Rust file.
///
/// The crate root is the nearest ancestor with a `Cargo.toml`; the owning README is the
/// first `README.md` met while walking from the file's directory up to that root.
pub(crate) fn resolve_ownership(rust_file: &Path) -> Result<Ownership, String> {
    let mut owning_readme = None;
    let mut directory = rust_file.parent();
    while let Some(current) = directory {
        let readme = current.join("README.md");
        if owning_readme.is_none() && readme.is_file() {
            owning_readme = Some(readme);
        }
        if current.join("Cargo.toml").is_file() {
            return Ok(Ownership {
                crate_root: current.to_path_buf(),
                owning_readme,
            });
        }
        directory = current.parent();
    }
    Err(format!("no Cargo.toml found above {}", rust_file.display()))
}

/// README that a `seg::seg::id` qualifier points at: `<crate>/src/<segments>/README.md`.
pub(crate) fn qualified_readme(crate_root: &Path, segments: &[String]) -> PathBuf {
    let mut path = crate_root.join("src");
    for segment in segments {
        path.push(segment);
    }
    path.join("README.md")
}
