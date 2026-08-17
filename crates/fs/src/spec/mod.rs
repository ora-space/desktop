use crate::search::{
    MAX_SEARCH_OUTPUT_BYTES, MAX_SEARCH_RESULTS, SEARCH_TIMEOUT, collect_process_output,
    normalize_ripgrep_path,
};
use crate::workspace::{canonical_root, relative_string, resolve_existing};
use crate::{ReadFile, WorkspaceFileSystem, WorkspaceFileSystemError};
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, ProcessStdio};
use ora_utils::path::CanonicalPathRoot;
use std::collections::BTreeMap;
use std::path::Path;

/// Describes one bounded Markdown file discovered below a workspace root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownFile {
    pub path: String,
    pub size_bytes: u64,
}

/// Returns an ordered Markdown index and whether a configured resource boundary was reached.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownIndex {
    pub files: Vec<MarkdownFile>,
    pub truncated: bool,
}

impl<S> WorkspaceFileSystem<S>
where
    S: ProcessSpawner,
{
    /// Discovers Markdown while honoring ignore files and excluding common generated directories.
    pub async fn discover_spec_markdown(
        &self,
        root: &Path,
    ) -> Result<MarkdownIndex, WorkspaceFileSystemError> {
        self.enumerate_markdown(root, &[], IgnorePolicy::Honor)
            .await
    }

    /// Enumerates explicit source directories even when Git ignore rules suppress them globally.
    pub async fn enumerate_spec_sources(
        &self,
        root: &Path,
        relative_sources: &[String],
    ) -> Result<MarkdownIndex, WorkspaceFileSystemError> {
        self.enumerate_markdown(root, relative_sources, IgnorePolicy::Bypass)
            .await
    }

    /// Reads only Markdown or MDX through the existing bounded, symlink-safe text reader.
    pub fn read_spec_file(
        &self,
        root: &Path,
        relative_path: &Path,
    ) -> Result<ReadFile, WorkspaceFileSystemError> {
        if !is_markdown_path(relative_path) {
            return Err(WorkspaceFileSystemError::NotFile {
                path: relative_path.to_path_buf(),
            });
        }
        self.read_file(root, relative_path)
    }

    /// Runs the shared ripgrep process with spec-specific globs and bounded output collection.
    async fn enumerate_markdown(
        &self,
        root: &Path,
        relative_sources: &[String],
        ignore_policy: IgnorePolicy,
    ) -> Result<MarkdownIndex, WorkspaceFileSystemError> {
        let root = canonical_root(root)?;
        let mut scopes = Vec::new();
        for source in relative_sources {
            let resolved = match resolve_existing(&root, Path::new(source)) {
                Ok(resolved) if resolved.is_dir() => resolved,
                Ok(_) | Err(WorkspaceFileSystemError::PathNotFound { .. }) => continue,
                Err(error) => return Err(error),
            };
            scopes.push(relative_string(&root, &resolved)?);
        }
        if !relative_sources.is_empty() && scopes.is_empty() {
            return Ok(MarkdownIndex {
                files: Vec::new(),
                truncated: false,
            });
        }

        let arguments = markdown_arguments(&scopes, ignore_policy);
        let spec = ProcessSpec::new(self.ripgrep_path.as_os_str())
            .args(arguments)
            .cwd(root.as_path())
            .stdin(ProcessStdio::Null);
        let mut process = self.process_spawner.spawn(spec).map_err(|source| {
            WorkspaceFileSystemError::SearchToolUnavailable {
                path: self.ripgrep_path.clone(),
                source,
            }
        })?;
        let stdout =
            process
                .take_stdout()
                .ok_or_else(|| WorkspaceFileSystemError::SearchFailed {
                    message: "ripgrep stdout pipe is unavailable".to_string(),
                })?;
        let stderr =
            process
                .take_stderr()
                .ok_or_else(|| WorkspaceFileSystemError::SearchFailed {
                    message: "ripgrep stderr pipe is unavailable".to_string(),
                })?;
        let completion = collect_process_output(&process, stdout, stderr);
        let (status, mut stdout, stderr) =
            match tokio::time::timeout(SEARCH_TIMEOUT, completion).await {
                Ok(result) => result?,
                Err(_) => {
                    let _ = process.kill().await;
                    return Err(WorkspaceFileSystemError::SearchTimedOut);
                }
            };
        let output_truncated = stdout.len() > MAX_SEARCH_OUTPUT_BYTES;
        stdout.truncate(MAX_SEARCH_OUTPUT_BYTES);
        if output_truncated
            && let Some(last_line_end) = stdout.iter().rposition(|byte| *byte == b'\n')
        {
            stdout.truncate(last_line_end + 1);
        }
        if stderr.len() > MAX_SEARCH_OUTPUT_BYTES {
            return Err(WorkspaceFileSystemError::SearchOutputTooLarge {
                limit_bytes: MAX_SEARCH_OUTPUT_BYTES,
            });
        }
        if !status.success() && status.code() != Some(1) {
            return Err(WorkspaceFileSystemError::SearchFailed {
                message: String::from_utf8_lossy(&stderr).trim().to_string(),
            });
        }
        parse_markdown_output(&root, &stdout, output_truncated, MAX_SEARCH_RESULTS)
    }
}

#[derive(Clone, Copy)]
enum IgnorePolicy {
    Honor,
    Bypass,
}

/// Builds fixed ripgrep arguments; only already-contained source paths occupy value positions.
fn markdown_arguments(scopes: &[String], ignore_policy: IgnorePolicy) -> Vec<String> {
    let mut arguments = vec![
        "--files".to_string(),
        "--hidden".to_string(),
        "--iglob".to_string(),
        "*.md".to_string(),
        "--iglob".to_string(),
        "*.mdx".to_string(),
        "--glob".to_string(),
        "!**/.git/**".to_string(),
        "--glob".to_string(),
        "!**/node_modules/**".to_string(),
        "--glob".to_string(),
        "!**/target/**".to_string(),
    ];
    if matches!(ignore_policy, IgnorePolicy::Bypass) {
        arguments.push("--no-ignore".to_string());
    }
    arguments.push("--".to_string());
    if scopes.is_empty() {
        arguments.push(".".to_string());
    } else {
        arguments.extend(scopes.iter().cloned());
    }
    arguments
}

/// Parses bounded ripgrep output, deduplicates overlaps, and obtains authoritative file sizes.
fn parse_markdown_output(
    root: &CanonicalPathRoot,
    output: &[u8],
    mut truncated: bool,
    max_results: usize,
) -> Result<MarkdownIndex, WorkspaceFileSystemError> {
    let mut files = BTreeMap::new();
    for line in String::from_utf8_lossy(output).lines() {
        let relative = normalize_ripgrep_path(root, line)?;
        if files.len() == max_results && !files.contains_key(&relative) {
            truncated = true;
            break;
        }
        let resolved = match resolve_existing(root, Path::new(&relative)) {
            Ok(resolved) => resolved,
            // Files can disappear between ripgrep output and metadata collection, and
            // workspace-local symlinks may point outside the security boundary. Neither
            // condition should make unrelated Specs unavailable.
            Err(WorkspaceFileSystemError::PathNotFound { .. })
            | Err(WorkspaceFileSystemError::PathOutsideWorkspace { .. }) => continue,
            Err(error) => return Err(error),
        };
        if !is_markdown_path(&resolved) || !resolved.is_file() {
            continue;
        }
        let size_bytes = match resolved.metadata() {
            Ok(metadata) => metadata.len(),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(WorkspaceFileSystemError::Io {
                    path: resolved,
                    source,
                });
            }
        };
        files.insert(relative, size_bytes);
    }
    Ok(MarkdownIndex {
        files: files
            .into_iter()
            .map(|(path, size_bytes)| MarkdownFile { path, size_bytes })
            .collect(),
        truncated,
    })
}

/// Accepts Markdown extensions case-insensitively without treating other text files as Specs.
fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("mdx")
        })
}

#[cfg(test)]
mod tests {
    use super::{IgnorePolicy, WorkspaceFileSystem, markdown_arguments, parse_markdown_output};
    use ora_process::TokioProcessSpawner;
    use ora_utils::path::CanonicalPathRoot;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// Verifies discovery and explicit enumeration share fixed generated-directory exclusions.
    #[test]
    fn builds_bounded_markdown_arguments() {
        let discovery = markdown_arguments(&[], IgnorePolicy::Honor);
        let explicit = markdown_arguments(&["docs/specs".to_string()], IgnorePolicy::Bypass);

        assert!(
            discovery
                .windows(2)
                .any(|pair| pair == ["--iglob", "*.mdx"])
        );
        assert!(discovery.contains(&"!**/node_modules/**".to_string()));
        assert!(!discovery.contains(&"--no-ignore".to_string()));
        assert!(explicit.contains(&"--no-ignore".to_string()));
        assert_eq!(explicit.last(), Some(&"docs/specs".to_string()));
    }

    /// Verifies global discovery honors Git ignore while explicit sources bypass it.
    #[tokio::test]
    async fn discovers_case_insensitive_markdown_and_explicit_ignored_sources() {
        let workspace = TempDir::new().expect("create temporary workspace");
        write_file(workspace.path(), "docs/specs/design.MDX", "# Design\n");
        write_file(workspace.path(), "ignored/specs/private.md", "# Private\n");
        fs::create_dir(workspace.path().join(".git")).expect("create Git marker");
        fs::write(workspace.path().join(".gitignore"), "ignored/\n").expect("write ignore fixture");
        let file_system = WorkspaceFileSystem::new(PathBuf::from("rg"), TokioProcessSpawner::new());

        assert_eq!(
            file_system
                .discover_spec_markdown(workspace.path())
                .await
                .expect("discover Markdown"),
            super::MarkdownIndex {
                files: vec![super::MarkdownFile {
                    path: "docs/specs/design.MDX".to_string(),
                    size_bytes: 9,
                }],
                truncated: false,
            }
        );
        assert_eq!(
            file_system
                .enumerate_spec_sources(workspace.path(), &["ignored/specs".to_string()],)
                .await
                .expect("enumerate ignored source"),
            super::MarkdownIndex {
                files: vec![super::MarkdownFile {
                    path: "ignored/specs/private.md".to_string(),
                    size_bytes: 10,
                }],
                truncated: false,
            }
        );
    }

    /// Verifies Spec reads reject non-Markdown files and symbolic-link escapes.
    #[test]
    fn reads_only_contained_markdown() {
        let workspace = TempDir::new().expect("create temporary workspace");
        let outside = TempDir::new().expect("create outside directory");
        write_file(workspace.path(), "docs/specs/design.md", "# Design\n");
        write_file(workspace.path(), "docs/specs/notes.txt", "notes\n");
        write_file(outside.path(), "outside.md", "outside\n");
        let file_system = WorkspaceFileSystem::new(PathBuf::from("rg"), TokioProcessSpawner::new());

        assert_eq!(
            file_system
                .read_spec_file(workspace.path(), Path::new("docs/specs/design.md"))
                .expect("read Markdown")
                .content,
            "# Design\n"
        );
        assert!(matches!(
            file_system.read_spec_file(workspace.path(), Path::new("docs/specs/notes.txt")),
            Err(crate::WorkspaceFileSystemError::NotFile { .. })
        ));

        let link = workspace
            .path()
            .join("docs")
            .join("specs")
            .join("escape.md");
        if create_file_symlink(&outside.path().join("outside.md"), &link).is_ok() {
            assert!(matches!(
                file_system.read_spec_file(workspace.path(), Path::new("docs/specs/escape.md")),
                Err(crate::WorkspaceFileSystemError::PathOutsideWorkspace { .. })
            ));
        }
    }

    /// Verifies result and byte-boundary truncation remain visible to catalog callers.
    #[test]
    fn reports_markdown_index_truncation() {
        let workspace = TempDir::new().expect("create temporary workspace");
        for name in ["a.md", "b.md", "c.md"] {
            write_file(workspace.path(), name, name);
        }
        let output = b"a.md\nb.md\nc.md\n";
        let root = CanonicalPathRoot::new(workspace.path())
            .unwrap_or_else(|error| panic!("canonicalize workspace: {error}"));

        assert_eq!(
            parse_markdown_output(&root, output, false, 2).expect("parse bounded output"),
            super::MarkdownIndex {
                files: vec![
                    super::MarkdownFile {
                        path: "a.md".to_string(),
                        size_bytes: 4,
                    },
                    super::MarkdownFile {
                        path: "b.md".to_string(),
                        size_bytes: 4,
                    },
                ],
                truncated: true,
            }
        );
        assert!(
            parse_markdown_output(&root, b"a.md\n", true, 2)
                .expect("parse byte-truncated output")
                .truncated
        );
    }

    /// Writes one fixture while creating its parent directories.
    fn write_file(root: &Path, relative_path: &str, content: &str) {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture parent");
        fs::write(path, content).expect("write fixture");
    }

    /// Creates a platform-native file symlink when the test environment permits it.
    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    /// Creates a Windows file symlink when Developer Mode or privileges permit it.
    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }
}
