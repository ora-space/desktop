use std::path::{Component, Path, PathBuf};

/// Normalizes an absolute path lexically so comparisons do not depend on filesystem existence.
///
/// `.` components are dropped and `..` pops the previous component; because an absolute path
/// cannot traverse above its root, surplus `..` segments at the top are ignored rather than
/// reported. No symlink resolution happens here; use
/// [`canonicalize_longest_existing_prefix`] when the existing part of the path must be resolved.
pub fn normalize_absolute(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    normalized
}

/// Normalizes a relative path lexically, returning `None` when it is rooted or `..` would escape.
///
/// This is the lexical counterpart of a containment check for paths that may not exist yet: any
/// prefix or root component, or a `..` that pops past the start, means the path cannot be
/// contained under the caller's root.
pub fn normalize_relative(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    let mut depth = 0usize;

    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => return None,
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return None;
                }

                let popped = normalized.pop();
                if popped {
                    depth -= 1;
                }
            }
            Component::Normal(part) => {
                normalized.push(part);
                depth += 1;
            }
        }
    }

    Some(normalized)
}

/// Canonicalizes the longest existing prefix of a path and re-appends the missing suffix.
///
/// Comparing paths reported by external tools (Git, the OS) against caller input needs the
/// existing part resolved through the filesystem (symlinks, Windows short names) while still
/// tolerating trailing components that do not exist yet. When nothing on the path exists the
/// result falls back to plain lexical normalization.
pub fn canonicalize_longest_existing_prefix(path: &Path) -> PathBuf {
    let mut current = path;
    let mut suffix_parts = Vec::new();

    loop {
        if let Ok(canonical_path) = std::fs::canonicalize(current) {
            let mut normalized = normalize_absolute(&canonical_path);
            for suffix_part in suffix_parts.iter().rev() {
                normalized.push(suffix_part);
            }

            return normalize_absolute(&normalized);
        }

        match (current.parent(), current.file_name()) {
            (Some(parent), Some(file_name)) => {
                suffix_parts.push(file_name.to_os_string());
                current = parent;
            }
            _ => return normalize_absolute(path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_longest_existing_prefix, normalize_absolute, normalize_relative};
    use pretty_assertions::assert_eq;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    #[test]
    fn normalizes_absolute_paths_lexically() {
        let input = Path::new("/")
            .join("a")
            .join(".")
            .join("b")
            .join("..")
            .join("c");
        assert_eq!(
            normalize_absolute(&input),
            Path::new("/").join("a").join("c")
        );

        // Surplus `..` at the root is dropped because nothing sits above the root.
        let escaping = Path::new("/").join("..").join("x");
        assert_eq!(normalize_absolute(&escaping), Path::new("/").join("x"));
    }

    #[test]
    fn normalizes_relative_paths_and_rejects_escapes() {
        let input = Path::new("a").join("b").join("..").join("c");
        assert_eq!(normalize_relative(&input), Some(Path::new("a").join("c")));
        assert_eq!(normalize_relative(Path::new(".")), Some(PathBuf::new()));
        assert_eq!(normalize_relative(&Path::new("..").join("x")), None);
        assert_eq!(
            normalize_relative(&Path::new("a").join("..").join("..")),
            None
        );
        assert_eq!(normalize_relative(&Path::new("/").join("rooted")), None);
    }

    #[test]
    fn canonicalizes_existing_prefix_and_keeps_missing_suffix() {
        let root = TempDir::new().unwrap_or_else(|error| panic!("create root: {error}"));
        let canonical_root = root
            .path()
            .canonicalize()
            .unwrap_or_else(|error| panic!("canonicalize root: {error}"));
        let candidate = root.path().join("missing").join(".").join("leaf");

        assert_eq!(
            canonicalize_longest_existing_prefix(&candidate),
            canonical_root.join("missing").join("leaf")
        );
    }
}
