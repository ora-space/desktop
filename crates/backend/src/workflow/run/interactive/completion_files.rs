//! Evaluates a node's frozen `completion.files` declaration against the live workspace.
//!
//! This check runs on every completion attempt, not at park: the workspace is mutable and a user
//! who fills a missing file expects the next click to succeed without re-parking the node. The
//! resulting blockers are returned to the caller and never written into `payload.completion`.

use super::super::completion_snapshot::{
    CompletionBlocker, CompletionBlockerKind, FILE_CHECK_UNAVAILABLE,
};
use super::super::worktree::{
    WorktreeFile, capture_declared_worktree_files, capture_worktree_snapshot,
    load_worktree_baseline,
};
use ora_application::AgentCompletion;
use ora_domain::WorkflowNodeRunId;
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;

/// The result of one live completion-file check.
pub(crate) struct CompletionFileCheck {
    pub blockers: Vec<CompletionBlocker>,
    /// Checks that could not be judged (legacy binary baseline, ignored files, oversized snapshot).
    pub warnings: Vec<String>,
}

/// Returns one blocker per declaration whose existence or `mustChange` requirement fails.
///
/// Existence and change comparison use two observations: the regular diff snapshot for every
/// workspace file it covers, and a targeted capture of declared files that Git ignores or that an
/// oversized sibling would hide. Both are contained and never read an escaping link. `mustChange`
/// degrades to `file_check_unavailable` only when a legacy baseline cannot prove a binary file's
/// prior content; a missing or corrupt baseline is handled where the node parks.
pub(crate) fn declared_file_blockers(
    completion: Option<&AgentCompletion>,
    node_run_id: &WorkflowNodeRunId,
    baselines_root: &Path,
    worktree_root: &Path,
) -> CompletionFileCheck {
    let Some(completion) = completion else {
        return CompletionFileCheck {
            blockers: Vec::new(),
            warnings: Vec::new(),
        };
    };
    let current = capture_worktree_snapshot(worktree_root);
    let declared_current = capture_declared_worktree_files(worktree_root, completion);
    let baseline = load_worktree_baseline(baselines_root, node_run_id);
    let contained = ContainedFiles::new(worktree_root);
    let mut blockers = Vec::new();
    let mut warnings = BTreeSet::new();
    for file in &completion.files {
        let mut matching: BTreeSet<String> = declared_current
            .keys()
            .filter(|path| file.matches(path))
            .cloned()
            .collect();
        if let Some(current) = current.as_ref() {
            matching.extend(
                current
                    .keys()
                    .filter(|path| contained.contains(path) && file.matches(path))
                    .cloned(),
            );
        }
        if matching.is_empty() {
            blockers.push(file_missing(file.path()));
            continue;
        }
        if !file.must_change {
            continue;
        }
        let Some(baseline) = baseline.as_ref() else {
            // The park snapshot already carries `file_check_unavailable`; only existence is judged.
            continue;
        };
        let mut changed = false;
        let mut undecidable = false;
        for path in &matching {
            let after = declared_current
                .get(path)
                .and_then(Option::as_ref)
                .or_else(|| {
                    current
                        .as_ref()
                        .and_then(|current| current.get(path))
                        .and_then(Option::as_ref)
                });
            let Some(after) = after else {
                continue;
            };
            let before = baseline.files.get(path).and_then(Option::as_ref);
            // Old baselines stored every binary file as `null`; without a digest the prior content
            // is unknowable, so this declaration cannot claim either a change or no change.
            if baseline.legacy && before.is_none() && matches!(after, WorktreeFile::Binary { .. }) {
                undecidable = true;
                continue;
            }
            if before != Some(after) {
                changed = true;
                break;
            }
        }
        if changed {
            continue;
        }
        if undecidable {
            warnings.insert(FILE_CHECK_UNAVAILABLE.to_string());
            continue;
        }
        blockers.push(CompletionBlocker {
            kind: CompletionBlockerKind::FileUnchanged,
            params: json!({ "path": file.path() }),
        });
    }
    CompletionFileCheck {
        blockers,
        warnings: warnings.into_iter().collect(),
    }
}

/// Builds the missing-file blocker whose param is the original declaration.
fn file_missing(path: &str) -> CompletionBlocker {
    CompletionBlocker {
        kind: CompletionBlockerKind::FileMissing,
        params: json!({ "path": path }),
    }
}

/// Filters workspace snapshot entries down to regular files inside the workspace.
///
/// The snapshot already resolves containment before reading, but checking again here keeps the
/// completion decision independent of how the snapshot was produced: a symbolic link that escapes
/// the workspace, a directory, or a gitlink must never satisfy a file declaration.
struct ContainedFiles {
    root: Option<CanonicalPathRoot>,
    contained: RefCell<BTreeSet<String>>,
}

impl ContainedFiles {
    /// Captures the canonical root once; an unavailable root contains no matching file.
    fn new(worktree_root: &Path) -> Self {
        Self {
            root: CanonicalPathRoot::new(worktree_root).ok(),
            contained: RefCell::new(BTreeSet::new()),
        }
    }

    /// Returns whether this entry resolves to an existing regular file under the workspace.
    fn contains(&self, path: &String) -> bool {
        let Some(root) = self.root.as_ref() else {
            return false;
        };
        if self.contained.borrow().contains(path) {
            return true;
        }
        // Git stores `/` separators; a literal backslash filename is ambiguous with a portable
        // path and is never treated as a matching deliverable.
        if path.contains('\\') {
            return false;
        }
        let Ok(relative) = PortableRelativePath::parse(path) else {
            return false;
        };
        let contained = root.resolve_existing(&relative).is_ok_and(|resolved| {
            std::fs::metadata(resolved).is_ok_and(|metadata| metadata.is_file())
        });
        if contained {
            self.contained.borrow_mut().insert(path.clone());
        }
        contained
    }
}
