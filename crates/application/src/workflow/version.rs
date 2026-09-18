/// Longest caller-provided version the host accepts, in bytes.
///
/// A published version becomes one URL path segment and one row in the version list, so this
/// bound keeps a version addressable rather than guarding a storage limit.
pub(crate) const MAX_VERSION_BYTES: usize = 128;

/// The reserved version string naming the editable draft snapshot.
///
/// No published version may carry this spelling, because a snapshot addressed as `"draft"` is the
/// mutable workspace rather than a frozen version.
pub(crate) const DRAFT_VERSION: &str = "draft";

/// Returns whether a caller-provided version is safe to address as a single URL path segment.
///
/// The reserved `"draft"` spelling is deliberately not part of this predicate, so publish can
/// report it as its own error instead of a generic malformed version. Callers deciding whether a
/// version can be used at all want [`is_publishable_version`] rather than this.
pub(crate) fn is_valid_user_version(version: &str) -> bool {
    !version.trim().is_empty()
        && version.len() <= MAX_VERSION_BYTES
        && !matches!(version, "." | "..")
        && !version
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
}

/// Returns whether a caller-provided version can be published as-is.
///
/// This adds the reserved-word rule to [`is_valid_user_version`]. Callers that cannot repair an
/// unusable version — import derives one from a file name and otherwise falls back to an
/// automatic version — need the combined answer, because publishing under `"draft"` would be
/// refused outright rather than with a usable fallback.
pub(crate) fn is_publishable_version(version: &str) -> bool {
    version != DRAFT_VERSION && is_valid_user_version(version)
}

#[cfg(test)]
mod tests {
    use super::{DRAFT_VERSION, MAX_VERSION_BYTES, is_publishable_version, is_valid_user_version};

    #[test]
    fn accepts_ordinary_versions_and_rejects_unaddressable_ones() {
        for version in ["v1.0.0", "1.0.0", "中文版本", "a"] {
            assert!(is_valid_user_version(version), "{version} should be valid");
        }

        for version in ["", "   ", ".", "..", "a/b", "a\\b", "a\tb", "a\nb"] {
            assert!(
                !is_valid_user_version(version),
                "{version:?} should be invalid"
            );
        }
    }

    #[test]
    fn bounds_versions_by_bytes() {
        assert!(is_valid_user_version(&"a".repeat(MAX_VERSION_BYTES)));
        assert!(!is_valid_user_version(&"a".repeat(MAX_VERSION_BYTES + 1)));
    }

    #[test]
    fn only_the_reserved_word_separates_publishable_from_addressable() {
        assert!(is_valid_user_version(DRAFT_VERSION));
        assert!(!is_publishable_version(DRAFT_VERSION));
        assert!(is_publishable_version("v1.0.0"));
        assert!(!is_publishable_version(" "));
    }
}
