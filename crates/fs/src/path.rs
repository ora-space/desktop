use std::path::{Component, Path, PathBuf};
use thiserror::Error;

/// Holds a platform-independent relative path in slash-separated canonical form.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PortableRelativePath {
    normalized: String,
}

impl PortableRelativePath {
    /// Parses untrusted wire or configuration input without platform-dependent path semantics.
    pub fn parse(value: &str) -> Result<Self, PortableRelativePathError> {
        if value.contains('\0') {
            return Err(PortableRelativePathError::NulByte);
        }
        if has_windows_prefix(value) {
            return Err(PortableRelativePathError::WindowsPrefix);
        }
        if value.starts_with(['/', '\\']) {
            return Err(PortableRelativePathError::Rooted);
        }

        let mut segments = Vec::new();
        for segment in value.split(['/', '\\']) {
            match segment {
                "" | "." => {}
                ".." => return Err(PortableRelativePathError::ParentTraversal),
                segment if has_windows_prefix(segment) => {
                    return Err(PortableRelativePathError::WindowsPrefix);
                }
                segment if is_windows_reserved_device_name(segment) => {
                    return Err(PortableRelativePathError::WindowsReservedName);
                }
                segment => segments.push(segment),
            }
        }

        Ok(Self {
            normalized: segments.join("/"),
        })
    }

    /// Returns whether this path denotes the containing root rather than one of its descendants.
    pub fn is_root(&self) -> bool {
        self.normalized.is_empty()
    }

    /// Returns the stable slash-separated representation used by contracts and persistence.
    pub fn as_str(&self) -> &str {
        &self.normalized
    }

    /// Converts the portable representation into a path using host-native components.
    pub fn to_path_buf(&self) -> PathBuf {
        self.normalized
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect()
    }

    /// Builds a portable path from an already-relative host path.
    fn from_host_path(path: &Path) -> Result<Self, PathContainmentError> {
        let mut segments = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(segment) => {
                    let segment =
                        segment
                            .to_str()
                            .ok_or_else(|| PathContainmentError::NonUtf8Path {
                                path: path.to_path_buf(),
                            })?;
                    // Rechecking host components keeps this private constructor aligned with
                    // `parse`, so every construction path preserves the public type invariant.
                    if segment.contains(['/', '\\'])
                        || has_windows_prefix(segment)
                        || is_windows_reserved_device_name(segment)
                    {
                        return Err(PathContainmentError::NonPortablePath {
                            path: path.to_path_buf(),
                        });
                    }
                    segments.push(segment);
                }
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(PathContainmentError::NonCanonicalPath {
                        path: path.to_path_buf(),
                    });
                }
            }
        }

        Ok(Self {
            normalized: segments.join("/"),
        })
    }
}

impl AsRef<str> for PortableRelativePath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Describes why portable relative-path parsing rejected an input.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PortableRelativePathError {
    #[error("relative path must not be rooted")]
    Rooted,
    #[error("relative path must not contain a Windows drive or UNC prefix")]
    WindowsPrefix,
    #[error("relative path must not contain a Windows reserved device name")]
    WindowsReservedName,
    #[error("relative path must not contain parent traversal")]
    ParentTraversal,
    #[error("relative path must not contain a NUL byte")]
    NulByte,
}

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

/// Detects Windows drive and UNC prefixes consistently on every host platform.
fn has_windows_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with("//")
        || value.starts_with("\\\\")
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

/// Detects device names that Win32 resolves as devices even when they have an extension.
fn is_windows_reserved_device_name(segment: &str) -> bool {
    let stem = segment
        .split_once('.')
        .map_or(segment, |(stem, _)| stem)
        .trim_end_matches([' ', '.']);

    if ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        return true;
    }

    let bytes = stem.as_bytes();
    bytes.len() == 4
        && (bytes[..3].eq_ignore_ascii_case(b"COM") || bytes[..3].eq_ignore_ascii_case(b"LPT"))
        && matches!(bytes[3], b'1'..=b'9')
}

#[cfg(test)]
mod tests {
    use super::{
        CanonicalPathRoot, PathContainmentError, PortableRelativePath, PortableRelativePathError,
    };
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Verifies portable parsing has identical normalization semantics on every host platform.
    #[test]
    fn normalizes_portable_relative_paths() {
        let cases = [
            ("", ""),
            (".", ""),
            ("./", ""),
            ("docs\\specs//./api", "docs/specs/api"),
        ];

        for (input, expected) in cases {
            assert_eq!(
                PortableRelativePath::parse(input)
                    .unwrap_or_else(|error| panic!("parse {input:?}: {error}"))
                    .as_str(),
                expected
            );
        }
    }

    /// Verifies traversal and platform-specific absolute spellings are rejected everywhere.
    #[test]
    fn rejects_unsafe_portable_relative_paths() {
        let cases = [
            ("../secret", PortableRelativePathError::ParentTraversal),
            ("docs/../secret", PortableRelativePathError::ParentTraversal),
            ("/rooted", PortableRelativePathError::Rooted),
            ("\\rooted", PortableRelativePathError::Rooted),
            ("C:\\rooted", PortableRelativePathError::WindowsPrefix),
            ("C:relative", PortableRelativePathError::WindowsPrefix),
            ("safe/C:/outside", PortableRelativePathError::WindowsPrefix),
            ("safe/C:\\outside", PortableRelativePathError::WindowsPrefix),
            ("//server/share", PortableRelativePathError::WindowsPrefix),
            (
                "\\\\server\\share",
                PortableRelativePathError::WindowsPrefix,
            ),
            ("bad\0path", PortableRelativePathError::NulByte),
        ];

        for (input, expected) in cases {
            assert_eq!(PortableRelativePath::parse(input), Err(expected), "{input}");
        }
    }

    /// Verifies Windows reserved device names are unsafe in every portable path component.
    #[test]
    fn rejects_windows_reserved_device_names() {
        let cases = [
            "CON",
            "con.txt",
            "folder/PrN.md",
            "AUX.tar.gz",
            "NUL.",
            "COM1",
            "com9.log",
            "folder\\LPT1\\notes.txt",
            "lpt9.md",
        ];

        for input in cases {
            assert_eq!(
                PortableRelativePath::parse(input),
                Err(PortableRelativePathError::WindowsReservedName),
                "{input}"
            );
        }
    }

    /// Verifies names outside the reserved device set remain portable ordinary paths.
    #[test]
    fn accepts_names_near_windows_reserved_device_names() {
        let cases = [
            ("CONSOLE.txt", "CONSOLE.txt"),
            ("COM0", "COM0"),
            ("COM10.txt", "COM10.txt"),
            ("LPT0", "LPT0"),
            ("LPT10.md", "LPT10.md"),
            ("NULLED/SKILL.md", "NULLED/SKILL.md"),
            ("folder.CON/notes.txt", "folder.CON/notes.txt"),
        ];

        for (input, expected) in cases {
            assert_eq!(
                PortableRelativePath::parse(input)
                    .unwrap_or_else(|error| panic!("parse {input:?}: {error}"))
                    .as_str(),
                expected
            );
        }
    }

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

    /// Verifies lexical parent components cannot bypass the portable-path parser invariant.
    #[test]
    fn rejects_noncanonical_relative_conversion() {
        let noncanonical = Path::new("nested").join("..").join("outside");

        assert!(matches!(
            PortableRelativePath::from_host_path(&noncanonical),
            Err(PathContainmentError::NonCanonicalPath { .. })
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
