//! Worktree baseline capture and the file-change projection derived from it.
//!
//! Separated from node dispatch because these are provenance, not execution: a node runs to
//! completion whether or not a baseline could be taken, and every failure here degrades to an
//! empty diff rather than to a failed node. Keeping that rule in one place is what stops a
//! missing snapshot from being read as an empty worktree in which every file is new.

use super::executor::NodeExecutionError;
use ora_application::{AgentCompletion, FileChange};
use ora_domain::WorkflowNodeRunId;
use ora_utils::hash::sha256_hex;
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

/// Total bytes of tracked-file content a worktree baseline may hold before capture aborts.
///
/// A baseline is execution provenance, not a prerequisite for running a node: exceeding this cap
/// aborts the snapshot and the node continues with an empty diff, rather than copying an unbounded
/// amount of repository content to disk.
const MAX_BASELINE_BYTES: u64 = 64 * 1024 * 1024;

/// A declared file larger than this is digested instead of loaded whole, so an explicit
/// declaration never turns a completion check into an unbounded allocation.
const MAX_INLINE_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// One file's captured content, represented without assuming text.
///
/// The untagged wire form keeps old baselines readable: a string is text and `null` (the outer
/// `Option`) is a missing/deleted file. Binary files store a digest so two different bytes are a
/// change even though neither can produce a line diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum WorktreeFile {
    /// UTF-8 content, which still supports the existing line-count presentation.
    Text(String),
    /// Non-UTF-8 content identified by its SHA-256 digest.
    Binary { sha256: String },
}

impl WorktreeFile {
    /// Counts display lines for a file, using one unit for binary content.
    fn line_count(&self) -> u64 {
        match self {
            Self::Text(content) => content.lines().count() as u64,
            Self::Binary { .. } => 1,
        }
    }
}

/// One loaded baseline plus whether it predates binary digests.
///
/// Older builds stored every non-UTF-8 file as `null` because they only read text. A `null` entry
/// in such a baseline cannot prove a binary file is unchanged, so the completion check must
/// degrade that declaration to a warning instead of inventing a comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeBaseline {
    pub files: BTreeMap<String, Option<WorktreeFile>>,
    pub legacy: bool,
}

/// Persisted wire shape; the untagged legacy arm keeps pre-digest baselines readable.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum StoredWorktreeBaseline {
    Legacy(BTreeMap<String, Option<String>>),
    Versioned {
        version: u32,
        files: BTreeMap<String, Option<WorktreeFile>>,
    },
}

/// New baselines carry an explicit version so legacy `null` entries can be told apart.
#[derive(Serialize)]
struct PersistedWorktreeBaseline<'a> {
    version: u32,
    files: &'a BTreeMap<String, Option<WorktreeFile>>,
}

/// Captures every worktree-visible regular file (tracked and untracked, gitignore-respecting) as
/// worktree-relative path → content, or `None` when git/root resolution is unavailable or the
/// content exceeds [`MAX_BASELINE_BYTES`].
///
/// Unlike `git status --porcelain`, which folds untracked directories into a single `?? dir/`
/// entry and omits clean tracked files, `git ls-files -co` expands both: a node that creates
/// files inside a new directory (e.g. `openspec/...`) or edits an already-committed file for the
/// first time still shows up in the before/after delta. `None` deliberately means "no baseline is
/// available", never "the tree was empty", so callers report no diff instead of treating the whole
/// tree as new.
///
/// Every candidate is resolved against a canonical workspace root *before* it is read, so a
/// symbolic link that escapes the workspace never causes outside content to be read. Directories
/// and gitlinks are not files and are left out rather than being counted as an existing file.
pub(crate) fn capture_worktree_snapshot(
    worktree_root: &Path,
) -> Option<BTreeMap<String, Option<WorktreeFile>>> {
    let root = CanonicalPathRoot::new(worktree_root).ok()?;
    let mut command = Command::new("git");
    command
        .args(["ls-files", "-co", "--exclude-standard", "-z"])
        .current_dir(worktree_root);
    ora_utils::process::hide_console_window(&mut command);
    let Ok(output) = command.output() else {
        return None;
    };
    let mut snapshot = BTreeMap::new();
    let mut total_bytes: u64 = 0;
    for path in String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|entry| !entry.is_empty())
    {
        match capture_entry(&root, path, &mut total_bytes) {
            Ok(Some(entry)) => {
                snapshot.insert(path.to_string(), entry);
            }
            Ok(None) => {}
            // Exceeding the cap aborts the whole snapshot; a partially captured tree would make
            // unrelated paths look added or deleted.
            Err(()) => return None,
        }
    }
    Some(snapshot)
}

/// Captures the baseline a completion check needs: the normal diff snapshot plus declared files
/// that Git ignores or that an oversized sibling would otherwise hide.
///
/// A declared-only baseline is still useful when the full snapshot exceeds its budget: the
/// declaration names the files the author cares about, and those are captured directly.
pub(crate) fn capture_completion_baseline(
    worktree_root: &Path,
    completion: Option<&AgentCompletion>,
) -> Option<BTreeMap<String, Option<WorktreeFile>>> {
    let snapshot = capture_worktree_snapshot(worktree_root);
    let Some(completion) = completion else {
        return snapshot;
    };
    let declared = capture_declared_worktree_files(worktree_root, completion);
    match snapshot {
        Some(mut snapshot) => {
            snapshot.extend(declared);
            Some(snapshot)
        }
        // An empty declared map is still a fact: none of the declared files existed at start.
        None => Some(declared),
    }
}

/// Captures every currently existing file matching one declared completion pattern.
///
/// Candidate paths come from Git (tracked/untracked non-ignored plus ignored files), so a declared
/// ignored output is visible without walking unrelated build directories. Each match is resolved
/// and read individually, so an escaping link or directory never becomes a captured deliverable.
pub(crate) fn capture_declared_worktree_files(
    worktree_root: &Path,
    completion: &AgentCompletion,
) -> BTreeMap<String, Option<WorktreeFile>> {
    let Ok(root) = CanonicalPathRoot::new(worktree_root) else {
        return BTreeMap::new();
    };
    let mut captured = BTreeMap::new();
    for path in declared_candidate_files(worktree_root) {
        if !completion.files.iter().any(|file| file.matches(&path)) {
            continue;
        }
        if let Some(content) = capture_declared_file(&root, &path) {
            captured.insert(path, Some(content));
        }
    }
    captured
}

/// Lists tracked, untracked and ignored paths Git can see in the workspace.
fn declared_candidate_files(worktree_root: &Path) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for args in [
        &["ls-files", "-co", "--exclude-standard", "-z"][..],
        &["ls-files", "-o", "-i", "--exclude-standard", "-z"][..],
    ] {
        let mut command = Command::new("git");
        command.args(args).current_dir(worktree_root);
        ora_utils::process::hide_console_window(&mut command);
        let Ok(output) = command.output() else {
            continue;
        };
        for path in String::from_utf8_lossy(&output.stdout)
            .split('\0')
            .filter(|entry| !entry.is_empty())
        {
            paths.insert(path.to_string());
        }
    }
    paths
}

/// Reads one declared file, using a streaming digest when the file is too large to load inline.
fn capture_declared_file(root: &CanonicalPathRoot, path: &str) -> Option<WorktreeFile> {
    let relative = PortableRelativePath::parse(path).ok()?;
    let resolved = root.resolve_existing(&relative).ok()?;
    let metadata = std::fs::metadata(&resolved).ok()?;
    if !metadata.is_file() {
        return None;
    }
    if metadata.len() > MAX_INLINE_FILE_BYTES {
        return ora_utils::hash::sha256_file(&resolved)
            .ok()
            .map(|sha256| WorktreeFile::Binary { sha256 });
    }
    let bytes = std::fs::read(resolved).ok()?;
    Some(match String::from_utf8(bytes) {
        Ok(content) => WorktreeFile::Text(content),
        Err(error) => WorktreeFile::Binary {
            sha256: sha256_hex(error.as_bytes()),
        },
    })
}

/// Resolves and captures one git-listed path.
///
/// `Ok(Some(None))` records a deleted tracked path so the before/after delta can report it;
/// `Ok(None)` omits something that is not a workspace regular file (an escaping link, directory,
/// gitlink, or unreadable path), which makes a declaration naming it fail its existence check
/// instead of following something outside the workspace. `Err(())` aborts the whole snapshot when
/// the byte cap is exceeded.
fn capture_entry(
    root: &CanonicalPathRoot,
    path: &str,
    total_bytes: &mut u64,
) -> Result<Option<Option<WorktreeFile>>, ()> {
    // Git stores `/` separators; a literal backslash filename is ambiguous with a portable path.
    if path.contains('\\') {
        return Ok(None);
    }
    let Ok(relative) = PortableRelativePath::parse(path) else {
        return Ok(None);
    };
    let resolved = match root.resolve_existing(&relative) {
        Ok(resolved) => resolved,
        // A path that no longer exists is a tracked deletion and still belongs in the delta.
        Err(ora_utils::path::PathContainmentError::PathNotFound { .. }) => {
            return Ok(Some(None));
        }
        Err(_) => return Ok(None),
    };
    let Ok(metadata) = std::fs::metadata(&resolved) else {
        return Ok(None);
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    *total_bytes = total_bytes.saturating_add(metadata.len());
    if *total_bytes > MAX_BASELINE_BYTES {
        return Err(());
    }
    let Ok(bytes) = std::fs::read(&resolved) else {
        // An unreadable file is not a file we can judge; omit it rather than record a fake state.
        return Ok(None);
    };
    Ok(Some(match String::from_utf8(bytes) {
        Ok(content) => Some(WorktreeFile::Text(content)),
        Err(error) => Some(WorktreeFile::Binary {
            sha256: sha256_hex(error.as_bytes()),
        }),
    }))
}

/// Diffs the worktree state captured before a node ran against the state after it finished, so
/// only this node's incremental changes are reported. A missing baseline or current snapshot
/// yields an empty diff — it is never treated as an empty worktree that makes every file new.
pub(crate) fn compute_file_changes(
    baseline: Option<&BTreeMap<String, Option<WorktreeFile>>>,
    current: Option<&BTreeMap<String, Option<WorktreeFile>>>,
) -> Vec<FileChange> {
    let (Some(baseline), Some(current)) = (baseline, current) else {
        return Vec::new();
    };
    let paths: BTreeSet<&String> = baseline.keys().chain(current.keys()).collect();
    let mut changes = Vec::new();
    for path in paths {
        let before = baseline.get(path).and_then(Clone::clone);
        let after = current.get(path).and_then(Clone::clone);
        let (additions, deletions) = match (before, after) {
            (None, Some(after)) => (after.line_count(), 0),
            (Some(before), None) => (0, before.line_count()),
            (Some(before), Some(after)) if before == after => (0, 0),
            (Some(before), Some(after)) => line_diff_counts(&before, &after),
            (None, None) => continue,
        };
        if additions > 0 || deletions > 0 {
            changes.push(FileChange {
                path: path.clone(),
                additions,
                deletions,
            });
        }
    }
    changes
}

/// Counts the added and removed lines between two captured files.
///
/// Text content keeps the existing line presentation. A binary side cannot be line-diffed, so a
/// changed binary reports one addition/deletion for the file rather than pretending a line count.
fn line_diff_counts(before: &WorktreeFile, after: &WorktreeFile) -> (u64, u64) {
    let (WorktreeFile::Text(before), WorktreeFile::Text(after)) = (before, after) else {
        return (1, 1);
    };
    let diff = TextDiff::from_lines(before, after);
    let additions = diff
        .iter_all_changes()
        .filter(|change| change.tag() == ChangeTag::Insert)
        .count() as u64;
    let deletions = diff
        .iter_all_changes()
        .filter(|change| change.tag() == ChangeTag::Delete)
        .count() as u64;
    (additions, deletions)
}

/// Returns the side file that stores one node run's start-of-node worktree baseline.
pub(crate) fn worktree_baseline_path(
    baselines_root: &Path,
    node_run_id: &WorkflowNodeRunId,
) -> std::path::PathBuf {
    baselines_root.join(format!("{}.json", node_run_id.as_ref()))
}

/// Loads the worktree baseline persisted when an interactive node started.
///
/// A missing, unreadable, or malformed file means the node's changes cannot be attributed; callers
/// degrade to reporting no changes or skipping `mustChange` rather than treating the whole tree as
/// changed. Old string/null baselines remain readable through [`WorktreeFile`]'s untagged form.
pub(crate) fn load_worktree_baseline(
    baselines_root: &Path,
    node_run_id: &WorkflowNodeRunId,
) -> Option<WorktreeBaseline> {
    let bytes = std::fs::read(worktree_baseline_path(baselines_root, node_run_id)).ok()?;
    match serde_json::from_slice::<StoredWorktreeBaseline>(&bytes).ok()? {
        StoredWorktreeBaseline::Legacy(files) => Some(WorktreeBaseline {
            files: files
                .into_iter()
                .map(|(path, content)| (path, content.map(WorktreeFile::Text)))
                .collect(),
            legacy: true,
        }),
        StoredWorktreeBaseline::Versioned { version: 1, files } => Some(WorktreeBaseline {
            files,
            legacy: false,
        }),
        // A future version is not guessed at; callers treat it as unavailable.
        StoredWorktreeBaseline::Versioned { .. } => None,
    }
}

/// Reports whether a baseline can actually be parsed and used for change attribution.
///
/// This is intentionally the same parse the completion check performs: a file that merely exists
/// but is corrupt must produce the `file_check_unavailable` warning instead of silently skipping
/// `mustChange` in a park that looked healthy.
pub(crate) fn worktree_baseline_available(
    baselines_root: &Path,
    node_run_id: &WorkflowNodeRunId,
) -> bool {
    load_worktree_baseline(baselines_root, node_run_id).is_some()
}

/// Persists the worktree snapshot captured when an interactive node started, so the completion
/// flow can later diff only this node's changes. Lives under a dedicated baselines root, never
/// the database or the worktree itself.
pub(super) fn persist_worktree_baseline(
    baselines_root: &Path,
    node_run_id: &WorkflowNodeRunId,
    baseline: &BTreeMap<String, Option<WorktreeFile>>,
) -> Result<(), NodeExecutionError> {
    std::fs::create_dir_all(baselines_root).map_err(|source| {
        NodeExecutionError::BaselinePersist {
            node_id: node_run_id.to_string(),
            source,
        }
    })?;
    let path = worktree_baseline_path(baselines_root, node_run_id);
    let json = serde_json::to_vec(&PersistedWorktreeBaseline {
        version: 1,
        files: baseline,
    })
    .map_err(|error| NodeExecutionError::BaselinePersist {
        node_id: node_run_id.to_string(),
        source: error.into(),
    })?;
    std::fs::write(path, json).map_err(|source| NodeExecutionError::BaselinePersist {
        node_id: node_run_id.to_string(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_application::FileChange;
    use pretty_assertions::assert_eq;

    /// Builds a text entry for fixture maps.
    fn text(value: &str) -> Option<WorktreeFile> {
        Some(WorktreeFile::Text(value.to_string()))
    }

    /// A missing baseline or current snapshot yields an empty diff, never "the whole tree is new".
    #[test]
    fn compute_file_changes_reports_empty_diff_when_a_side_is_missing() {
        let mut baseline = BTreeMap::new();
        baseline.insert("src/a.ts".to_string(), text("one\n"));
        assert_eq!(compute_file_changes(None, Some(&baseline)), Vec::new());
        assert_eq!(compute_file_changes(Some(&baseline), None), Vec::new());
    }

    /// Text content keeps the existing incremental line-diff presentation.
    #[test]
    fn compute_file_changes_reports_only_the_incremental_delta() {
        let mut baseline = BTreeMap::new();
        baseline.insert("src/a.ts".to_string(), text("one\ntwo\n"));
        baseline.insert("src/b.ts".to_string(), text("keep\n"));

        let mut current = BTreeMap::new();
        current.insert("src/a.ts".to_string(), text("one\ntwo\nthree\n"));
        current.insert("src/b.ts".to_string(), None);
        current.insert("src/new.ts".to_string(), text("fresh\n"));

        // a.ts gained a line, b.ts was deleted, new.ts was added; keep unchanged is excluded.
        assert_eq!(
            compute_file_changes(Some(&baseline), Some(&current)),
            vec![
                FileChange {
                    path: "src/a.ts".to_string(),
                    additions: 1,
                    deletions: 0
                },
                FileChange {
                    path: "src/b.ts".to_string(),
                    additions: 0,
                    deletions: 1
                },
                FileChange {
                    path: "src/new.ts".to_string(),
                    additions: 1,
                    deletions: 0
                },
            ]
        );
    }

    /// Binary changes are reported as a changed file even when both sides are non-UTF-8.
    #[test]
    fn compute_file_changes_detects_binary_edits() {
        let first = WorktreeFile::Binary {
            sha256: sha256_hex(b"\x00\x01"),
        };
        let second = WorktreeFile::Binary {
            sha256: sha256_hex(b"\x00\x02"),
        };
        let mut baseline = BTreeMap::new();
        baseline.insert("asset.bin".to_string(), Some(first.clone()));
        let mut current = BTreeMap::new();
        current.insert("asset.bin".to_string(), Some(second));
        assert_eq!(
            compute_file_changes(Some(&baseline), Some(&current)),
            vec![FileChange {
                path: "asset.bin".to_string(),
                additions: 1,
                deletions: 1
            }]
        );
        // Identical binary content is not a change.
        current.insert("asset.bin".to_string(), Some(first));
        assert_eq!(
            compute_file_changes(Some(&baseline), Some(&current)),
            Vec::new()
        );
    }

    /// Verifies a persisted worktree baseline round-trips through its side file.
    #[test]
    fn persist_worktree_baseline_round_trips() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut baseline = BTreeMap::new();
        baseline.insert("src/a.ts".to_string(), text("one\n"));
        baseline.insert(
            "src/b.bin".to_string(),
            Some(WorktreeFile::Binary {
                sha256: sha256_hex(b"\x00\x01"),
            }),
        );
        baseline.insert("src/deleted.ts".to_string(), None);
        persist_worktree_baseline(temp.path(), &WorkflowNodeRunId::new("node-1"), &baseline)
            .unwrap();
        let loaded = load_worktree_baseline(temp.path(), &WorkflowNodeRunId::new("node-1"))
            .expect("baseline loads");
        assert_eq!(loaded.files, baseline);
        assert!(!loaded.legacy);
    }

    /// Old baselines stored text as a bare string and a deletion as null; both remain readable.
    #[test]
    fn old_baseline_shape_remains_readable() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            temp.path().join("node-1.json"),
            r#"{"src/a.ts":"one\n","src/gone.ts":null}"#,
        )
        .unwrap();
        let loaded = load_worktree_baseline(temp.path(), &WorkflowNodeRunId::new("node-1"))
            .expect("old baseline parses");
        assert!(loaded.legacy);
        assert_eq!(loaded.files.get("src/a.ts"), Some(&text("one\n")));
        assert_eq!(loaded.files.get("src/gone.ts"), Some(&None));
    }

    /// A corrupt baseline is unavailable even though its side file exists.
    #[test]
    fn corrupt_baseline_is_not_available() {
        let temp = tempfile::TempDir::new().unwrap();
        let node_run_id = WorkflowNodeRunId::new("node-1");
        std::fs::write(
            worktree_baseline_path(temp.path(), &node_run_id),
            "{not json",
        )
        .unwrap();
        assert!(!worktree_baseline_available(temp.path(), &node_run_id));
    }

    /// Verifies the completion-time diff against a baseline loaded from its side file reports
    /// only the node's own changes since the node started.
    #[test]
    fn compute_file_changes_against_a_loaded_baseline() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut baseline = BTreeMap::new();
        baseline.insert("src/a.ts".to_string(), text("one\n"));
        persist_worktree_baseline(temp.path(), &WorkflowNodeRunId::new("node-1"), &baseline)
            .unwrap();
        let loaded = load_worktree_baseline(temp.path(), &WorkflowNodeRunId::new("node-1"))
            .expect("baseline loads");

        let mut current = baseline;
        current.insert("src/a.ts".to_string(), text("one\ntwo\n"));
        current.insert("src/new.ts".to_string(), text("fresh\n"));
        assert_eq!(
            compute_file_changes(Some(&loaded.files), Some(&current)),
            vec![
                FileChange {
                    path: "src/a.ts".to_string(),
                    additions: 1,
                    deletions: 0
                },
                FileChange {
                    path: "src/new.ts".to_string(),
                    additions: 1,
                    deletions: 0
                },
            ]
        );
    }

    /// Verifies the snapshot covers clean tracked files and files inside untracked directories,
    /// so the before/after delta reports the node's own edits rather than whole-file additions.
    #[test]
    fn capture_worktree_snapshot_diffs_clean_tracked_and_untracked_dir_files() {
        let scaffold = ora_test_support::GitTestScaffold::new("backend-workflow-snapshot")
            .expect("create Git test scaffold");
        let root = scaffold.repo_path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        // A tracked file that is clean at the baseline and modified by the node.
        std::fs::write(root.join("src/a.ts"), "one\ntwo\n").unwrap();
        scaffold
            .stage_all_and_commit("init")
            .expect("create snapshot baseline commit");

        let baseline = capture_worktree_snapshot(root).expect("capture baseline");
        // The clean tracked file is part of the baseline.
        assert_eq!(baseline.get("src/a.ts"), Some(&text("one\ntwo\n")));

        // The node edits the tracked file and creates files inside a new untracked directory.
        std::fs::write(root.join("src/a.ts"), "one\ntwo\nthree\n").unwrap();
        std::fs::create_dir_all(root.join("openspec/changes/demo")).unwrap();
        std::fs::write(root.join("openspec/changes/demo/proposal.md"), "fresh\n").unwrap();

        assert_eq!(
            compute_file_changes(Some(&baseline), capture_worktree_snapshot(root).as_ref()),
            vec![
                FileChange {
                    path: "openspec/changes/demo/proposal.md".to_string(),
                    additions: 1,
                    deletions: 0
                },
                FileChange {
                    path: "src/a.ts".to_string(),
                    additions: 1,
                    deletions: 0
                },
            ]
        );
    }

    /// A symbolic link that escapes the workspace is not captured and its target is never read.
    #[cfg(unix)]
    #[test]
    fn capture_worktree_snapshot_excludes_an_escaping_symlink() {
        let scaffold = ora_test_support::GitTestScaffold::new("backend-workflow-symlink")
            .expect("create Git test scaffold");
        let root = scaffold.repo_path();
        let outside = tempfile::TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.bin"), b"\x00\x01").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.bin"), root.join("escape.bin"))
            .unwrap();
        let snapshot = capture_worktree_snapshot(root).expect("capture baseline");
        assert!(!snapshot.contains_key("escape.bin"));
    }

    /// A directory or gitlink may be listed by git but is not a file deliverable.
    #[test]
    fn capture_worktree_snapshot_excludes_directories() {
        let scaffold = ora_test_support::GitTestScaffold::new("backend-workflow-directory")
            .expect("create Git test scaffold");
        let root = scaffold.repo_path();
        std::fs::create_dir_all(root.join("reports")).unwrap();
        std::fs::write(root.join("reports/plan.md"), "plan\n").unwrap();
        let snapshot = capture_worktree_snapshot(root).expect("capture baseline");
        assert!(snapshot.contains_key("reports/plan.md"));
        assert!(!snapshot.contains_key("reports"));
    }
}
