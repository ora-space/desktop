//! Worktree baseline capture and the file-change projection derived from it.
//!
//! Separated from node dispatch because these are provenance, not execution: a node runs to
//! completion whether or not a baseline could be taken, and every failure here degrades to an
//! empty diff rather than to a failed node. Keeping that rule in one place is what stops a
//! missing snapshot from being read as an empty worktree in which every file is new.

use super::executor::NodeExecutionError;
use ora_application::FileChange;
use ora_domain::WorkflowNodeRunId;
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

/// Captures every worktree-visible file (tracked and untracked, gitignore-respecting) as
/// worktree-relative path → content, or `None` when git is unavailable or the content exceeds
/// [`MAX_BASELINE_BYTES`].
///
/// Unlike `git status --porcelain`, which folds untracked directories into a single `?? dir/`
/// entry and omits clean tracked files, `git ls-files -co` expands both: a node that creates
/// files inside a new directory (e.g. `openspec/...`) or edits an already-committed file for the
/// first time still shows up in the before/after delta. `None` deliberately means "no baseline is
/// available", never "the tree was empty", so callers report no diff instead of treating the whole
/// tree as new.
pub(crate) fn capture_worktree_snapshot(
    worktree_root: &Path,
) -> Option<BTreeMap<String, Option<String>>> {
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
        // Check the file's size before reading it, so a single oversized file cannot be read whole
        // into memory before the cap trips. A deleted tracked file has no metadata and reads as
        // `None`; a directory entry can only be an in-index submodule, which we skip as content.
        let Ok(metadata) = std::fs::metadata(worktree_root.join(path)) else {
            snapshot.insert(path.to_string(), None);
            continue;
        };
        total_bytes = total_bytes.saturating_add(metadata.len());
        if total_bytes > MAX_BASELINE_BYTES {
            return None;
        }
        let content = std::fs::read_to_string(worktree_root.join(path)).ok();
        snapshot.insert(path.to_string(), content);
    }
    Some(snapshot)
}

/// Diffs the worktree state captured before a node ran against the state after it finished, so
/// only this node's incremental changes are reported. A missing baseline or current snapshot
/// yields an empty diff — it is never treated as an empty worktree that makes every file new.
pub(crate) fn compute_file_changes(
    baseline: Option<&BTreeMap<String, Option<String>>>,
    current: Option<&BTreeMap<String, Option<String>>>,
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
            (None, Some(after)) => (count_lines(&after), 0),
            (Some(before), None) => (0, count_lines(&before)),
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

/// Counts the added and removed lines between two file contents.
fn line_diff_counts(before: &str, after: &str) -> (u64, u64) {
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

/// Counts the lines of a file for new-file additions or whole-file deletions.
fn count_lines(content: &str) -> u64 {
    content.lines().count() as u64
}

/// Persists the worktree snapshot captured when an interactive node started, so the completion
/// flow can later diff only this node's changes. Lives under a dedicated baselines root, never
/// the database or the worktree itself.
pub(super) fn persist_worktree_baseline(
    baselines_root: &Path,
    node_run_id: &WorkflowNodeRunId,
    baseline: &BTreeMap<String, Option<String>>,
) -> Result<(), NodeExecutionError> {
    std::fs::create_dir_all(baselines_root).map_err(|source| {
        NodeExecutionError::BaselinePersist {
            node_id: node_run_id.to_string(),
            source,
        }
    })?;
    let path = baselines_root.join(format!("{}.json", node_run_id.as_ref()));
    let json =
        serde_json::to_vec(baseline).map_err(|error| NodeExecutionError::BaselinePersist {
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

    /// A missing baseline or current snapshot yields an empty diff, never "the whole tree is new".
    #[test]
    fn compute_file_changes_reports_empty_diff_when_a_side_is_missing() {
        let mut baseline = BTreeMap::new();
        baseline.insert("src/a.ts".to_string(), Some("one\n".to_string()));
        assert_eq!(compute_file_changes(None, Some(&baseline)), Vec::new());
        assert_eq!(compute_file_changes(Some(&baseline), None), Vec::new());
    }
    #[test]
    fn compute_file_changes_reports_only_the_incremental_delta() {
        let mut baseline = BTreeMap::new();
        baseline.insert("src/a.ts".to_string(), Some("one\ntwo\n".to_string()));
        baseline.insert("src/b.ts".to_string(), Some("keep\n".to_string()));

        let mut current = BTreeMap::new();
        current.insert(
            "src/a.ts".to_string(),
            Some("one\ntwo\nthree\n".to_string()),
        );
        current.insert("src/b.ts".to_string(), None);
        current.insert("src/new.ts".to_string(), Some("fresh\n".to_string()));

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
    /// Verifies a persisted worktree baseline round-trips through its side file.
    #[test]
    fn persist_worktree_baseline_round_trips() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut baseline = BTreeMap::new();
        baseline.insert("src/a.ts".to_string(), Some("one\n".to_string()));
        baseline.insert("src/b.ts".to_string(), None);
        persist_worktree_baseline(temp.path(), &WorkflowNodeRunId::new("node-1"), &baseline)
            .unwrap();
        let loaded: BTreeMap<String, Option<String>> =
            serde_json::from_slice(&std::fs::read(temp.path().join("node-1.json")).unwrap())
                .unwrap();
        assert_eq!(loaded, baseline);
    }
    /// Verifies the completion-time diff against a baseline loaded from its side file reports
    /// only the node's own changes since the node started.
    #[test]
    fn compute_file_changes_against_a_loaded_baseline() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut baseline = BTreeMap::new();
        baseline.insert("src/a.ts".to_string(), Some("one\n".to_string()));
        persist_worktree_baseline(temp.path(), &WorkflowNodeRunId::new("node-1"), &baseline)
            .unwrap();
        let loaded: BTreeMap<String, Option<String>> =
            serde_json::from_slice(&std::fs::read(temp.path().join("node-1.json")).unwrap())
                .unwrap();

        let mut current = baseline;
        current.insert("src/a.ts".to_string(), Some("one\ntwo\n".to_string()));
        current.insert("src/new.ts".to_string(), Some("fresh\n".to_string()));
        assert_eq!(
            compute_file_changes(Some(&loaded), Some(&current)),
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
        assert_eq!(
            baseline.get("src/a.ts"),
            Some(&Some("one\ntwo\n".to_string()))
        );

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
}
