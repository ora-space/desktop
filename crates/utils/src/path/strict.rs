use super::portable::PortableRelativePath;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::fmt;
use std::path::{Path, PathBuf};

/// Length and depth limits applied while parsing one [`StrictRelativePath`].
///
/// The defaults are conservative cross-platform values: 255 bytes / 255 UTF-16 code units per
/// segment (the NTFS and most Unix filesystem component limit), 1024 bytes per full path, and 32
/// nested directories. Callers materializing untrusted trees inject their own values when a
/// different budget applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelativePathLimits {
    /// Maximum UTF-8 bytes allowed in one path segment.
    pub max_segment_bytes: usize,
    /// Maximum UTF-16 code units allowed in one path segment (Windows path component limit).
    pub max_segment_utf16_units: usize,
    /// Maximum UTF-8 bytes allowed in a full relative path.
    pub max_path_bytes: usize,
    /// Maximum nested directory depth allowed below the root.
    pub max_depth: usize,
}

impl Default for RelativePathLimits {
    fn default() -> Self {
        Self {
            max_segment_bytes: 255,
            max_segment_utf16_units: 255,
            max_path_bytes: 1024,
            max_depth: 32,
        }
    }
}

/// Reports why one raw relative path failed strict validation.
///
/// The error is deliberately safe to display: `EncodingInvalid` and `Unsafe` carry no
/// attacker-controlled path fragments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrictRelativePathError {
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

/// A strictly validated relative path stored with forward-slash separators.
///
/// Unlike [`PortableRelativePath`], parsing rejects every irregular spelling instead of
/// normalizing it: zip-slip, absolute, drive/UNC, traversal, empty or `.` segments, trailing
/// separators, Windows reserved device names, control characters, and non-UTF-8 all fail, and
/// segment, total-length, and depth limits apply. Instances are safe to use as key material and
/// to reconstruct under a destination root via [`StrictRelativePath::to_path`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StrictRelativePath {
    path: String,
}

impl Serialize for StrictRelativePath {
    /// Serializes the already-normalized portable spelling used as stable receipt data.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StrictRelativePath {
    /// Revalidates persisted or wire-provided paths so deserialization cannot bypass containment
    /// invariants.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(|_| de::Error::custom("invalid strict relative path"))
    }
}

impl StrictRelativePath {
    /// Returns the empty root directory used to model top-level entries.
    pub fn root() -> StrictRelativePath {
        StrictRelativePath {
            path: String::new(),
        }
    }

    /// Parses one raw path with the default limits, treating both `/` and `\` as separators.
    pub fn parse(raw: &str) -> Result<Self, StrictRelativePathError> {
        Self::parse_with_limits(raw, &RelativePathLimits::default())
    }

    /// Parses one raw path with caller-supplied limits, treating both `/` and `\` as separators.
    pub fn parse_with_limits(
        raw: &str,
        limits: &RelativePathLimits,
    ) -> Result<Self, StrictRelativePathError> {
        validate_control_characters(raw)?;

        let normalized = raw.replace('\\', "/");
        validate_shape(&normalized)?;
        // Reusing the portable parser keeps platform-specific filename safety consistent with
        // every other filesystem-facing caller while this type adds strictness and limits.
        PortableRelativePath::parse(&normalized).map_err(|_| StrictRelativePathError::Unsafe)?;
        let segments = normalized.split('/').collect::<Vec<_>>();

        let depth = segments.len() - 1;
        if depth > limits.max_depth {
            return Err(StrictRelativePathError::TooDeep);
        }
        if normalized.len() > limits.max_path_bytes {
            return Err(StrictRelativePathError::TooLong);
        }
        for segment in &segments {
            if segment.len() > limits.max_segment_bytes
                || segment.encode_utf16().count() > limits.max_segment_utf16_units
            {
                return Err(StrictRelativePathError::SegmentTooLong);
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

    /// Returns the parent directory path, or the root for top-level paths.
    pub fn parent(&self) -> Option<StrictRelativePath> {
        if self.path.is_empty() {
            return None;
        }
        match self.path.rfind('/') {
            Some(index) => Some(StrictRelativePath {
                path: self.path[..index].to_string(),
            }),
            None => Some(StrictRelativePath::root()),
        }
    }

    /// Appends one validated segment, producing the child path under this directory.
    pub fn append_segment(&self, segment: &str) -> StrictRelativePath {
        if self.path.is_empty() {
            StrictRelativePath {
                path: segment.to_string(),
            }
        } else {
            StrictRelativePath {
                path: format!("{}/{}", self.path, segment),
            }
        }
    }

    /// Strips a directory prefix from this path, returning the child-relative remainder.
    pub fn strip_prefix(&self, prefix: &StrictRelativePath) -> Option<StrictRelativePath> {
        if prefix.path.is_empty() {
            return Some(self.clone());
        }
        let remainder = self.path.strip_prefix(&format!("{}/", prefix.path))?;
        Some(StrictRelativePath {
            path: remainder.to_string(),
        })
    }

    /// Reconstructs the absolute filesystem path under a destination root.
    ///
    /// Each validated segment is joined through [`Path::join`] so no separator concatenation
    /// happens outside this type's virtual path model. The empty root resolves to the
    /// destination root itself.
    pub fn to_path(&self, root: &Path) -> PathBuf {
        if self.path.is_empty() {
            return root.to_path_buf();
        }
        let mut path = root.to_path_buf();
        for segment in self.path.split('/') {
            path = path.join(segment);
        }
        path
    }

    /// Builds the portable case-folded key used for whole-tree conflict detection.
    ///
    /// The path is Unicode NFC-normalized and then compared case-insensitively, matching the
    /// behaviour of case-insensitive filesystems. The original spelling stays authoritative for
    /// storage.
    pub fn fold_case_key(&self) -> String {
        use unicode_normalization::UnicodeNormalization;
        self.path.nfc().collect::<String>().to_ascii_lowercase()
    }
}

impl fmt::Display for StrictRelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.path)
    }
}

/// Rejects NUL and control characters that filesystems cannot represent safely.
fn validate_control_characters(raw: &str) -> Result<(), StrictRelativePathError> {
    if raw
        .chars()
        .any(|character| character.is_control() || character == '\u{7f}')
    {
        return Err(StrictRelativePathError::EncodingInvalid);
    }
    Ok(())
}

/// Rejects absolute, drive-letter, UNC, and traversal shapes before segment validation.
fn validate_shape(normalized: &str) -> Result<(), StrictRelativePathError> {
    let bytes = normalized.as_bytes();
    if bytes.starts_with(b"/") || normalized.contains("//") || looks_like_drive_or_unc(normalized) {
        return Err(StrictRelativePathError::Unsafe);
    }
    for segment in normalized.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(StrictRelativePathError::Unsafe);
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
    use super::{RelativePathLimits, StrictRelativePath, StrictRelativePathError};
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_plain_and_backslash_normalized_paths() {
        assert_eq!(
            StrictRelativePath::parse("a/b/SKILL.md").unwrap().as_str(),
            "a/b/SKILL.md"
        );
        assert_eq!(
            StrictRelativePath::parse("a\\b\\SKILL.md")
                .unwrap()
                .as_str(),
            "a/b/SKILL.md"
        );
        assert_eq!(
            StrictRelativePath::parse("a/b/SKILL.md")
                .unwrap()
                .file_name(),
            Some("SKILL.md")
        );
        assert_eq!(
            StrictRelativePath::parse("a/b/SKILL.md").unwrap().parent(),
            Some(StrictRelativePath::parse("a/b").unwrap())
        );
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
            ".",
            "..",
            "a/../b",
            "a/",
        ] {
            assert_eq!(
                StrictRelativePath::parse(raw),
                Err(StrictRelativePathError::Unsafe),
                "expected {raw:?} to be rejected"
            );
        }
    }

    /// Verifies strict paths inherit portable Windows device-name safety.
    #[test]
    fn rejects_windows_reserved_device_names() {
        for raw in ["CON", "folder/aux.txt", "folder\\COM1\\notes.txt"] {
            assert_eq!(
                StrictRelativePath::parse(raw),
                Err(StrictRelativePathError::Unsafe),
                "expected {raw:?} to be rejected"
            );
        }
    }

    /// Verifies the shared portable rule does not reject nearby legal path names.
    #[test]
    fn accepts_names_near_windows_reserved_device_names() {
        for raw in ["CONSOLE.txt", "COM10.txt", "folder/LPT0/notes.txt"] {
            assert_eq!(
                StrictRelativePath::parse(raw)
                    .unwrap_or_else(|error| panic!("parse {raw:?}: {error:?}"))
                    .as_str(),
                raw
            );
        }
    }

    #[test]
    fn rejects_non_utf8_and_control_characters() {
        assert_eq!(
            StrictRelativePath::parse("a/\u{0}/b"),
            Err(StrictRelativePathError::EncodingInvalid)
        );
        assert_eq!(
            StrictRelativePath::parse("a/\u{1f}/b"),
            Err(StrictRelativePathError::EncodingInvalid)
        );
    }

    #[test]
    fn enforces_segment_and_depth_limits() {
        let long_segment = "x".repeat(256);
        assert_eq!(
            StrictRelativePath::parse(&long_segment),
            Err(StrictRelativePathError::SegmentTooLong)
        );

        let wide_segment = "你".repeat(256);
        assert_eq!(
            StrictRelativePath::parse(&wide_segment),
            Err(StrictRelativePathError::SegmentTooLong)
        );

        let deep = std::iter::repeat_n("d", 34).collect::<Vec<_>>().join("/");
        assert_eq!(
            StrictRelativePath::parse(&deep),
            Err(StrictRelativePathError::TooDeep)
        );

        let within_depth = std::iter::repeat_n("d", 32).collect::<Vec<_>>().join("/");
        assert_eq!(
            StrictRelativePath::parse(&format!("{within_depth}/f"))
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
            StrictRelativePath::parse(&path),
            Err(StrictRelativePathError::TooLong)
        );
    }

    #[test]
    fn honours_injected_limits() {
        let limits = RelativePathLimits {
            max_segment_bytes: 3,
            max_segment_utf16_units: 3,
            max_path_bytes: 7,
            max_depth: 1,
        };
        assert_eq!(
            StrictRelativePath::parse_with_limits("abc/def", &limits)
                .unwrap()
                .as_str(),
            "abc/def"
        );
        assert_eq!(
            StrictRelativePath::parse_with_limits("abcd", &limits),
            Err(StrictRelativePathError::SegmentTooLong)
        );
        assert_eq!(
            StrictRelativePath::parse_with_limits("a/b/c", &limits),
            Err(StrictRelativePathError::TooDeep)
        );
        assert_eq!(
            StrictRelativePath::parse_with_limits("abc/defg", &limits),
            Err(StrictRelativePathError::TooLong)
        );
    }

    #[test]
    fn folds_case_after_unicode_normalization() {
        let composed = StrictRelativePath::parse("e\u{301}/file").unwrap();
        let decomposed = StrictRelativePath::parse("é/file").unwrap();
        assert_eq!(composed.fold_case_key(), decomposed.fold_case_key());
        assert_eq!(
            StrictRelativePath::parse("Review.md")
                .unwrap()
                .fold_case_key(),
            StrictRelativePath::parse("review.md")
                .unwrap()
                .fold_case_key()
        );
    }
}
