use super::portable::PortableRelativePath;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Owns one canonical filesystem root used for repeated containment checks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalPathRoot {
    path: PathBuf,
}

impl CanonicalPathRoot {
    /// Canonicalizes an existing root so all later comparisons use one stable filesystem identity.
    pub fn new(root: &Path) -> Result<Self, PathContainmentError> {
        let path = root
            .canonicalize()
            .map_err(|source| PathContainmentError::RootUnavailable {
                path: root.to_path_buf(),
                source,
            })?;
        Ok(Self { path })
    }

    /// Returns the canonical host path used as the containment boundary.
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    /// Resolves one existing portable path and verifies its canonical target remains contained.
    pub fn resolve_existing(
        &self,
        relative_path: &PortableRelativePath,
    ) -> Result<PathBuf, PathContainmentError> {
        let requested = self.path.join(relative_path.to_path_buf());
        self.resolve_candidate(&requested)
    }

    /// Resolves an existing absolute selection and verifies its canonical target remains contained.
    pub fn resolve_existing_absolute(
        &self,
        absolute_path: &Path,
    ) -> Result<PathBuf, PathContainmentError> {
        if !absolute_path.is_absolute() {
            return Err(PathContainmentError::PathNotAbsolute {
                path: absolute_path.to_path_buf(),
            });
        }
        self.resolve_candidate(absolute_path)
    }

    /// Converts an already-contained canonical path into its portable relative representation.
    pub fn relative_path(
        &self,
        canonical_path: &Path,
    ) -> Result<PortableRelativePath, PathContainmentError> {
        let relative = canonical_path.strip_prefix(&self.path).map_err(|_| {
            PathContainmentError::OutsideRoot {
                path: canonical_path.to_path_buf(),
            }
        })?;
        PortableRelativePath::from_host_path(relative)
    }

    /// Canonicalizes a candidate before checking containment under this root.
    fn resolve_candidate(&self, candidate: &Path) -> Result<PathBuf, PathContainmentError> {
        let resolved = candidate.canonicalize().map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                PathContainmentError::PathNotFound {
                    path: candidate.to_path_buf(),
                }
            } else {
                PathContainmentError::Io {
                    path: candidate.to_path_buf(),
                    source,
                }
            }
        })?;
        if !resolved.starts_with(&self.path) {
            return Err(PathContainmentError::OutsideRoot {
                path: candidate.to_path_buf(),
            });
        }

        // This check rejects the current symlink topology, but returning a path cannot prevent a
        // caller-controlled link from being replaced before the caller opens it (a TOCTOU race).
        Ok(resolved)
    }
}

/// Describes failures while establishing or applying a canonical path-containment boundary.
#[derive(Debug, Error)]
pub enum PathContainmentError {
    #[error("path root is unavailable: {path:?}")]
    RootUnavailable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("path must be absolute: {path:?}")]
    PathNotAbsolute { path: PathBuf },
    #[error("path was not found: {path:?}")]
    PathNotFound { path: PathBuf },
    #[error("path escapes its root: {path:?}")]
    OutsideRoot { path: PathBuf },
    #[error("path is not valid UTF-8: {path:?}")]
    NonUtf8Path { path: PathBuf },
    #[error("path contains a host filename without a safe portable representation: {path:?}")]
    NonPortablePath { path: PathBuf },
    #[error("path is not in canonical relative form: {path:?}")]
    NonCanonicalPath { path: PathBuf },
    #[error("filesystem operation failed for {path:?}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::{CanonicalPathRoot, PathContainmentError, PortableRelativePath};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Verifies canonical resolution returns stable contained paths and portable identities.
    #[test]
    fn resolves_existing_contained_paths() {
        let root = TempDir::new().unwrap_or_else(|error| panic!("create root: {error}"));
        fs::create_dir(root.path().join("docs"))
            .unwrap_or_else(|error| panic!("create docs: {error}"));
        fs::write(root.path().join("docs").join("spec.md"), "spec")
            .unwrap_or_else(|error| panic!("write spec: {error}"));
        let canonical_root = CanonicalPathRoot::new(root.path())
            .unwrap_or_else(|error| panic!("canonicalize root: {error}"));
        let relative = PortableRelativePath::parse("docs/spec.md")
            .unwrap_or_else(|error| panic!("parse path: {error}"));

        let resolved = canonical_root
            .resolve_existing(&relative)
            .unwrap_or_else(|error| panic!("resolve contained file: {error}"));

        assert_eq!(
            canonical_root
                .relative_path(&resolved)
                .unwrap_or_else(|error| panic!("make relative: {error}")),
            relative
        );
    }

    /// Verifies an absolute target outside the root is rejected after canonicalization.
    #[test]
    fn rejects_absolute_paths_outside_root() {
        let root = TempDir::new().unwrap_or_else(|error| panic!("create root: {error}"));
        let outside = TempDir::new().unwrap_or_else(|error| panic!("create outside root: {error}"));
        let canonical_root = CanonicalPathRoot::new(root.path())
            .unwrap_or_else(|error| panic!("canonicalize root: {error}"));

        assert!(matches!(
            canonical_root.resolve_existing_absolute(outside.path()),
            Err(PathContainmentError::OutsideRoot { .. })
        ));
    }

    /// Verifies relative absolute-selection input is rejected before filesystem access.
    #[test]
    fn rejects_relative_absolute_selection() {
        let root = TempDir::new().unwrap_or_else(|error| panic!("create root: {error}"));
        let canonical_root = CanonicalPathRoot::new(root.path())
            .unwrap_or_else(|error| panic!("canonicalize root: {error}"));

        assert!(matches!(
            canonical_root.resolve_existing_absolute(Path::new("docs")),
            Err(PathContainmentError::PathNotAbsolute { .. })
        ));
    }

    /// Verifies a Unix backslash filename cannot be mistaken for two portable path components.
    #[cfg(unix)]
    #[test]
    fn rejects_ambiguous_host_filename() {
        let root = TempDir::new().unwrap_or_else(|error| panic!("create root: {error}"));
        let path = root.path().join("one\\two");
        fs::write(&path, "content").unwrap_or_else(|error| panic!("write fixture: {error}"));
        let canonical_root = CanonicalPathRoot::new(root.path())
            .unwrap_or_else(|error| panic!("canonicalize root: {error}"));
        let canonical_path = path
            .canonicalize()
            .unwrap_or_else(|error| panic!("canonicalize fixture: {error}"));

        assert!(matches!(
            canonical_root.relative_path(&canonical_path),
            Err(PathContainmentError::NonPortablePath { .. })
        ));
    }
}
