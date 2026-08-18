use super::containment::PathContainmentError;
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
    pub(super) fn from_host_path(path: &Path) -> Result<Self, PathContainmentError> {
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
    use super::{PathContainmentError, PortableRelativePath, PortableRelativePathError};
    use pretty_assertions::assert_eq;
    use std::path::Path;

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

    /// Verifies lexical parent components cannot bypass the portable-path parser invariant.
    #[test]
    fn rejects_noncanonical_relative_conversion() {
        let noncanonical = Path::new("nested").join("..").join("outside");

        assert!(matches!(
            PortableRelativePath::from_host_path(&noncanonical),
            Err(PathContainmentError::NonCanonicalPath { .. })
        ));
    }
}
