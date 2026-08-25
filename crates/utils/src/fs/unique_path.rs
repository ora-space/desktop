use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Finds the first `stem.ext`, `stem-1.ext`, `stem-2.ext`, ... inside `directory` that neither
/// exists on disk nor is claimed by `occupied`.
///
/// The predicate exists because callers commonly hold reservations that are not visible on disk
/// yet (for example a download whose `.part` file is created later by the browser engine); the
/// disk check alone would hand out the same name twice.
pub fn next_available_file_name(
    directory: &Path,
    file_name: &str,
    occupied: impl Fn(&Path) -> bool,
) -> PathBuf {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or(file_name);
    let extension = Path::new(file_name).extension().and_then(OsStr::to_str);
    for index in 0u64.. {
        let candidate = match (index, extension) {
            (0, _) => file_name.to_owned(),
            (_, Some(extension)) => format!("{stem}-{index}.{extension}"),
            (_, None) => format!("{stem}-{index}"),
        };
        let path = directory.join(candidate);
        if !occupied(&path) && !path.exists() {
            return path;
        }
    }
    unreachable!("u64 exhaustion prevents allocating another file name")
}

#[cfg(test)]
mod tests {
    use super::next_available_file_name;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Verifies names skip files present on disk and names claimed by the caller's predicate.
    #[test]
    fn skips_existing_and_reserved_names() {
        let temporary = TempDir::new().expect("create temporary directory");
        let directory = temporary.path();
        fs::write(directory.join("skill.zip"), b"existing").expect("write existing file");
        let reserved = directory.join("skill-1.zip");

        let selected = next_available_file_name(directory, "skill.zip", |path| path == reserved);

        assert_eq!(selected, directory.join("skill-2.zip"));
    }

    /// Verifies extension-less and compound-extension names receive the suffix before the last
    /// extension only, as documented in the module README.
    #[test]
    fn places_suffix_before_last_extension() {
        let directory = Path::new("/virtual");
        let cases = [
            ("README", "README-1"),
            ("pack.tar.gz", "pack.tar-1.gz"),
            ("notes.txt", "notes-1.txt"),
        ];
        for (input, expected) in cases {
            let first = directory.join(input);
            assert_eq!(
                next_available_file_name(directory, input, |path| path == first),
                directory.join(expected),
                "{input}"
            );
        }
    }
}
