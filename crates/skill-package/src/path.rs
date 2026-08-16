use std::fmt;
use std::path::Path;

/// Maximum UTF-8 bytes and UTF-16 code units allowed in one path segment.
pub const MAX_SEGMENT_BYTES: usize = 255;
/// Maximum UTF-16 code units allowed in one path segment (Windows path component limit).
pub const MAX_SEGMENT_UTF16_UNITS: usize = 255;
/// Maximum UTF-8 bytes allowed in a full skill-relative path.
pub const MAX_PATH_BYTES: usize = 1024;
/// Maximum nested directory depth allowed below the source root.
pub const MAX_DEPTH: usize = 32;

/// Reports why one raw source path failed validation.
///
/// The error is deliberately safe to display: `EncodingInvalid` and `Unsafe` carry no
/// attacker-controlled path fragments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathValidationError {
    /// The path is not valid UTF-8 or contains disallowed control characters.
    EncodingInvalid,
    /// The path is absolute, a drive/UNC path, contains empty, `.`, or `..` segments, or names a
    /// Windows reserved device.
    Unsafe,
    /// One segment exceeds the byte or UTF-16 code-unit limit.
    SegmentTooLong,
    /// The full path exceeds the total byte limit.
    TooLong,
    /// The path nests deeper than the directory depth limit.
    TooDeep,
}

/// A validated skill-relative path stored with forward-slash separators.
///
/// Parsing rejects every unsafe shape (zip-slip, absolute, drive/UNC, traversal, Windows reserved
/// device names, control characters, non-UTF-8) and enforces segment, total-length, and depth
/// limits. Instances are safe to use as key material and to reconstruct under a destination root
/// via [`RelativePath::to_path`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelativePath {
    path: String,
}

impl RelativePath {
    /// Returns the empty source-root directory used to model top-level manifests.
    pub fn root() -> RelativePath {
        RelativePath {
            path: String::new(),
        }
    }

    /// Parses one raw source path, treating both `/` and `\` as separators.
    pub fn parse(raw: &str) -> Result<Self, PathValidationError> {
        validate_control_characters(raw)?;

        let normalized = raw.replace('\\', "/");
        validate_shape(&normalized)?;
        // Reusing the shared portable parser keeps platform-specific filename safety consistent
        // with every other filesystem-facing caller while this type adds package-specific limits.
        ora_fs::PortableRelativePath::parse(&normalized)
            .map_err(|_| PathValidationError::Unsafe)?;
        let segments = normalized.split('/').collect::<Vec<_>>();

        let depth = segments.len() - 1;
        if depth > MAX_DEPTH {
            return Err(PathValidationError::TooDeep);
        }
        if normalized.len() > MAX_PATH_BYTES {
            return Err(PathValidationError::TooLong);
        }
        for segment in &segments {
            if segment.len() > MAX_SEGMENT_BYTES
                || segment.encode_utf16().count() > MAX_SEGMENT_UTF16_UNITS
            {
                return Err(PathValidationError::SegmentTooLong);
            }
        }

        Ok(Self { path: normalized })
    }

    /// Returns the forward-slash normalized path value.
    pub fn as_str(&self) -> &str {
        &self.path
    }

    /// Returns the file-name segment of the path, if any.
    pub fn file_name(&self) -> Option<&str> {
        self.path.rsplit('/').next()
    }

    /// Returns the parent directory path, or the source root for top-level paths.
    pub fn parent(&self) -> Option<RelativePath> {
        if self.path.is_empty() {
            return None;
        }
        match self.path.rfind('/') {
            Some(index) => Some(RelativePath {
                path: self.path[..index].to_string(),
            }),
            None => Some(RelativePath::root()),
        }
    }

    /// Returns whether this path names the exact manifest file `SKILL.md`.
    pub fn is_manifest(&self) -> bool {
        self.file_name() == Some("SKILL.md")
    }

    /// Appends one validated segment, producing the child path under this directory.
    pub fn append_segment(&self, segment: &str) -> RelativePath {
        if self.path.is_empty() {
            RelativePath {
                path: segment.to_string(),
            }
        } else {
            RelativePath {
                path: format!("{}/{}", self.path, segment),
            }
        }
    }

    /// Strips a directory prefix from this path, returning the child-relative remainder.
    ///
    /// Used to map a skill-boundary file back to its position inside a staged skill directory.
    pub fn strip_prefix(&self, prefix: &RelativePath) -> Option<RelativePath> {
        if prefix.path.is_empty() {
            return Some(self.clone());
        }
        let remainder = self.path.strip_prefix(&format!("{}/", prefix.path))?;
        Some(RelativePath {
            path: remainder.to_string(),
        })
    }

    /// Reconstructs the absolute filesystem path under a destination root.
    ///
    /// Each validated segment is joined through [`std::path::Path::join`] so no separator
    /// concatenation happens outside this crate's virtual path model. The empty source root
    /// resolves to the destination root itself.
    pub fn to_path(&self, root: &Path) -> std::path::PathBuf {
        if self.path.is_empty() {
            return root.to_path_buf();
        }
        let mut path = root.to_path_buf();
        for segment in self.path.split('/') {
            path = path.join(segment);
        }
        path
    }

    /// Builds the portable case-folded key used for whole-source conflict detection.
    ///
    /// The path is Unicode NFC-normalized and then compared case-insensitively, matching the
    /// portable conflict rule. The original spelling stays authoritative for storage.
    pub fn fold_case_key(&self) -> String {
        use unicode_normalization::UnicodeNormalization;
        self.path.nfc().collect::<String>().to_ascii_lowercase()
    }
}

impl fmt::Display for RelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.path)
    }
}

/// Rejects NUL and control characters that filesystems cannot represent safely.
fn validate_control_characters(raw: &str) -> Result<(), PathValidationError> {
    if raw
        .chars()
        .any(|character| character.is_control() || character == '\u{7f}')
    {
        return Err(PathValidationError::EncodingInvalid);
    }
    Ok(())
}

/// Rejects absolute, drive-letter, UNC, and traversal shapes before segment validation.
fn validate_shape(normalized: &str) -> Result<(), PathValidationError> {
    let bytes = normalized.as_bytes();
    if bytes.starts_with(b"/") || normalized.contains("//") || looks_like_drive_or_unc(normalized) {
        return Err(PathValidationError::Unsafe);
    }
    for segment in normalized.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(PathValidationError::Unsafe);
        }
    }
    Ok(())
}

/// Detects a leading drive letter (`C:`) or UNC-style rooted path prefix.
fn looks_like_drive_or_unc(normalized: &str) -> bool {
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return true;
    }
    // A `\\server\share` path normalizes to `//server/share` and was already rejected by the
    // double-slash check; this guard keeps the classification explicit for callers.
    normalized.starts_with("//")
}

#[cfg(test)]
mod tests {
    use super::{PathValidationError, RelativePath};
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_plain_and_backslash_normalized_paths() {
        assert_eq!(
            RelativePath::parse("a/b/SKILL.md").unwrap().as_str(),
            "a/b/SKILL.md"
        );
        assert_eq!(
            RelativePath::parse("a\\b\\SKILL.md").unwrap().as_str(),
            "a/b/SKILL.md"
        );
        assert_eq!(
            RelativePath::parse("a/b/SKILL.md").unwrap().file_name(),
            Some("SKILL.md")
        );
        assert_eq!(
            RelativePath::parse("a/b/SKILL.md").unwrap().parent(),
            Some(RelativePath::parse("a/b").unwrap())
        );
        assert!(RelativePath::parse("a/b/SKILL.md").unwrap().is_manifest());
        assert!(!RelativePath::parse("a/b/skill.md").unwrap().is_manifest());
    }

    #[test]
    fn rejects_traversal_and_unsafe_shapes() {
        for raw in [
            "../escape",
            "a/../../escape",
            "a/./b",
            "/absolute",
            "a//b",
            "C:/drive",
            "c:\\windows",
            "//unc/share",
            "a//",
            "a//b",
            ".",
            "..",
            "a/../b",
            "a/",
        ] {
            assert_eq!(
                RelativePath::parse(raw),
                Err(PathValidationError::Unsafe),
                "expected {raw:?} to be rejected"
            );
        }
    }

    /// Verifies skill paths inherit portable Windows device-name safety before materialization.
    #[test]
    fn rejects_windows_reserved_device_names() {
        for raw in ["CON", "folder/aux.txt", "folder\\COM1\\notes.txt"] {
            assert_eq!(
                RelativePath::parse(raw),
                Err(PathValidationError::Unsafe),
                "expected {raw:?} to be rejected"
            );
        }
    }

    /// Verifies the shared portable rule does not reject nearby legal skill path names.
    #[test]
    fn accepts_names_near_windows_reserved_device_names() {
        for raw in ["CONSOLE.txt", "COM10.txt", "folder/LPT0/notes.txt"] {
            assert_eq!(
                RelativePath::parse(raw)
                    .unwrap_or_else(|error| panic!("parse {raw:?}: {error:?}"))
                    .as_str(),
                raw
            );
        }
    }

    #[test]
    fn rejects_non_utf8_and_control_characters() {
        assert_eq!(
            RelativePath::parse("a/\u{0}/b"),
            Err(PathValidationError::EncodingInvalid)
        );
        assert_eq!(
            RelativePath::parse("a/\u{1f}/b"),
            Err(PathValidationError::EncodingInvalid)
        );
    }

    #[test]
    fn enforces_segment_and_depth_limits() {
        let long_segment = "x".repeat(256);
        assert_eq!(
            RelativePath::parse(&long_segment),
            Err(PathValidationError::SegmentTooLong)
        );

        let wide_segment = "你".repeat(256);
        assert_eq!(
            RelativePath::parse(&wide_segment),
            Err(PathValidationError::SegmentTooLong)
        );

        let deep = std::iter::repeat_n("d", 34).collect::<Vec<_>>().join("/");
        assert_eq!(
            RelativePath::parse(&deep),
            Err(PathValidationError::TooDeep)
        );

        let within_depth = std::iter::repeat_n("d", 32).collect::<Vec<_>>().join("/");
        assert_eq!(
            RelativePath::parse(&format!("{within_depth}/f"))
                .unwrap()
                .as_str(),
            format!("{within_depth}/f")
        );
    }

    #[test]
    fn enforces_total_path_byte_limit() {
        let mut path = String::new();
        for index in 0..5 {
            if index > 0 {
                path.push('/');
            }
            path.push_str(&"y".repeat(250));
        }
        assert_eq!(
            RelativePath::parse(&path),
            Err(PathValidationError::TooLong)
        );
    }

    #[test]
    fn folds_case_after_unicode_normalization() {
        let composed = RelativePath::parse("e\u{301}/file").unwrap();
        let decomposed = RelativePath::parse("é/file").unwrap();
        assert_eq!(composed.fold_case_key(), decomposed.fold_case_key());
        assert_eq!(
            RelativePath::parse("Review.md").unwrap().fold_case_key(),
            RelativePath::parse("review.md").unwrap().fold_case_key()
        );
    }
}
