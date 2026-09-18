use std::path::{Path, PathBuf};

use gitlancer::git::worktree::FindWorktreeRequest;
use gitlancer::{
    ChangedSinceRequest, CliGitRunner, Git, RepoRoot, RestoreAllRequest, RestorePathsRequest,
    SnapshotWorktreeRequest,
};
use ora_test_support::GitTestScaffold as TestScaffold;
use pretty_assertions::assert_eq;

/// Creates an initial commit so checkpoint tests can start from a non-empty history.
fn seed_repository(scaffold: &TestScaffold) {
    scaffold
        .write_file(scaffold.repo_path(), "README.md", "seed repository\n")
        .expect("write seed file");
    scaffold
        .stage_all_and_commit("chore: seed repository")
        .expect("create initial commit");
}

/// Returns a typed runtime and the main worktree handle for one scaffold.
fn runtime_worktree(scaffold: &TestScaffold) -> (Git<CliGitRunner>, gitlancer::WorktreeHandle) {
    let git = Git::new(CliGitRunner);
    let repository = git
        .discover_repository(RepoRoot::new(scaffold.repo_path()))
        .expect("discover repository");
    let worktree = git
        .find_worktree(FindWorktreeRequest {
            repository: &repository,
            candidate_path: scaffold.repo_path(),
        })
        .expect("find main worktree");
    (git, worktree)
}

/// Reads the current HEAD commit, or `None` when the branch is still unborn.
fn head_commit(scaffold: &TestScaffold) -> Option<String> {
    scaffold
        .run_git(["rev-parse", "--verify", "--quiet", "HEAD"])
        .ok()
        .map(|oid| oid.trim().to_string())
        .filter(|oid| !oid.is_empty())
}

/// Asserts that the real index still matches HEAD (`git diff --cached --quiet`).
fn assert_real_index_untouched(scaffold: &TestScaffold) {
    scaffold
        .run_git(["diff", "--cached", "--quiet"])
        .expect("real index should remain unchanged");
}

/// Reads a worktree file as UTF-8 with newline normalization so Windows autocrlf does not fail tests.
fn read_normalized(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

/// Lists blob paths in a tree so tests can assert a checkpoint captured the expected files.
fn tree_paths(scaffold: &TestScaffold, tree_oid: &str) -> Vec<String> {
    let output = scaffold
        .run_git(["ls-tree", "-r", "--name-only", tree_oid])
        .expect("list tree paths");
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Verifies a snapshot of a committed repo with an untracked file and a modified tracked file.
#[test]
fn snapshot_worktree_captures_untracked_and_modified_files_without_touching_head_or_index() {
    let scaffold = TestScaffold::new("checkpoint-snapshot-dirty").expect("create scaffold");
    seed_repository(&scaffold);
    let head_before = head_commit(&scaffold).expect("seeded repository has HEAD");
    scaffold
        .write_file(
            scaffold.repo_path(),
            "README.md",
            "seed repository\nchanged\n",
        )
        .expect("modify tracked file");
    scaffold
        .write_file(scaffold.repo_path(), "untracked.txt", "new file\n")
        .expect("write untracked file");
    assert_real_index_untouched(&scaffold);
    let (git, worktree) = runtime_worktree(&scaffold);

    let response = git
        .snapshot_worktree(SnapshotWorktreeRequest {
            worktree: &worktree,
            name: "node-1",
            message: "ora checkpoint: dirty worktree",
        })
        .expect("snapshot dirty worktree");

    let resolved = scaffold
        .run_git(["rev-parse", "refs/ora/checkpoints/node-1"])
        .expect("resolve checkpoint ref");
    assert_eq!(resolved.trim(), response.commit_oid);
    let mut paths = tree_paths(&scaffold, &response.tree_oid);
    paths.sort();
    assert_eq!(
        paths,
        vec!["README.md".to_string(), "untracked.txt".to_string()]
    );
    assert_eq!(
        head_commit(&scaffold).as_deref(),
        Some(head_before.as_str())
    );
    assert_real_index_untouched(&scaffold);
}

/// Verifies a snapshot of an empty repository succeeds and writes a parentless commit.
#[test]
fn snapshot_worktree_succeeds_on_an_empty_repository() {
    let scaffold = TestScaffold::new("checkpoint-snapshot-empty").expect("create scaffold");
    scaffold
        .write_file(scaffold.repo_path(), "first.txt", "hello\n")
        .expect("write untracked file");
    let (git, worktree) = runtime_worktree(&scaffold);

    let response = git
        .snapshot_worktree(SnapshotWorktreeRequest {
            worktree: &worktree,
            name: "empty-node",
            message: "ora checkpoint: empty repo",
        })
        .expect("snapshot empty repository");

    let resolved = scaffold
        .run_git(["rev-parse", "refs/ora/checkpoints/empty-node"])
        .expect("resolve checkpoint ref");
    assert_eq!(resolved.trim(), response.commit_oid);
    assert!(
        scaffold
            .run_git([
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{}^", response.commit_oid)
            ])
            .is_err(),
        "empty-repo checkpoint must have no parent"
    );
    assert_eq!(head_commit(&scaffold), None);
    assert_real_index_untouched(&scaffold);
}

/// Verifies add/modify/delete after a checkpoint produce the matching statuses and counts.
#[test]
fn changed_since_reports_added_modified_and_deleted_paths() {
    let scaffold = TestScaffold::new("checkpoint-changed-since").expect("create scaffold");
    scaffold
        .write_file(scaffold.repo_path(), "keep.txt", "keep\n")
        .expect("write keep");
    scaffold
        .write_file(scaffold.repo_path(), "tracked.txt", "orig\n")
        .expect("write tracked");
    scaffold
        .write_file(scaffold.repo_path(), "gone.txt", "gone\n")
        .expect("write gone");
    scaffold
        .stage_all_and_commit("chore: seed tracked files")
        .expect("commit seed");
    let (git, worktree) = runtime_worktree(&scaffold);
    let snapshot = git
        .snapshot_worktree(SnapshotWorktreeRequest {
            worktree: &worktree,
            name: "before-edits",
            message: "ora checkpoint: before edits",
        })
        .expect("snapshot before edits");

    scaffold
        .write_file(scaffold.repo_path(), "added.txt", "add\n")
        .expect("add file");
    scaffold
        .write_file(scaffold.repo_path(), "tracked.txt", "orig\nmod\n")
        .expect("modify file");
    std::fs::remove_file(scaffold.repo_path().join("gone.txt")).expect("delete file");

    let mut entries = git
        .changed_since(ChangedSinceRequest {
            worktree: &worktree,
            commit_oid: &snapshot.commit_oid,
        })
        .expect("diff since checkpoint")
        .entries;
    entries.sort_by(|left, right| left.path.cmp(&right.path));

    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].path, "added.txt");
    assert_eq!(entries[0].status, gitlancer::ChangeStatus::Added);
    assert_eq!(entries[0].additions, Some(1));
    assert_eq!(entries[0].deletions, Some(0));
    assert_eq!(entries[1].path, "gone.txt");
    assert_eq!(entries[1].status, gitlancer::ChangeStatus::Deleted);
    assert_eq!(entries[1].additions, Some(0));
    assert_eq!(entries[1].deletions, Some(1));
    assert_eq!(entries[2].path, "tracked.txt");
    assert_eq!(entries[2].status, gitlancer::ChangeStatus::Modified);
    assert_eq!(entries[2].additions, Some(1));
    assert_eq!(entries[2].deletions, Some(0));
}

/// Restoring the modified and added paths leaves the deleted file deleted and the index untouched.
#[test]
fn restore_paths_restores_modified_and_deletes_added_without_touching_index() {
    let (scaffold, git, worktree, commit_oid) = dirty_after_checkpoint("checkpoint-restore-paths");

    git.restore_paths(RestorePathsRequest {
        worktree: &worktree,
        commit_oid: &commit_oid,
        paths: &["tracked.txt".to_string(), "added.txt".to_string()],
    })
    .expect("restore selected paths");

    assert_eq!(
        read_normalized(&scaffold.repo_path().join("tracked.txt")),
        "orig\n"
    );
    assert!(!scaffold.repo_path().join("added.txt").exists());
    assert!(!scaffold.repo_path().join("gone.txt").exists());
    assert_real_index_untouched(&scaffold);
}

/// Restoring every changed path returns the worktree to the checkpoint tree.
#[test]
fn restore_all_matches_the_checkpoint_tree_without_touching_index() {
    let (scaffold, git, worktree, commit_oid) = dirty_after_checkpoint("checkpoint-restore-all");

    git.restore_all(RestoreAllRequest {
        worktree: &worktree,
        commit_oid: &commit_oid,
    })
    .expect("restore all paths");

    let remaining = git
        .changed_since(ChangedSinceRequest {
            worktree: &worktree,
            commit_oid: &commit_oid,
        })
        .expect("diff after restore_all")
        .entries;
    assert_eq!(remaining, Vec::new());
    assert_eq!(
        read_normalized(&scaffold.repo_path().join("tracked.txt")),
        "orig\n"
    );
    assert_eq!(
        read_normalized(&scaffold.repo_path().join("gone.txt")),
        "gone\n"
    );
    assert!(!scaffold.repo_path().join("added.txt").exists());
    assert_real_index_untouched(&scaffold);
}

/// Restore rejects traversal and absolute paths before touching the worktree.
#[test]
fn restore_paths_rejects_parent_and_absolute_paths() {
    let scaffold = TestScaffold::new("checkpoint-restore-reject").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, worktree) = runtime_worktree(&scaffold);
    let snapshot = git
        .snapshot_worktree(SnapshotWorktreeRequest {
            worktree: &worktree,
            name: "reject",
            message: "ora checkpoint: reject paths",
        })
        .expect("snapshot");
    let absolute = PathBuf::from(if cfg!(windows) {
        r"C:\Windows\ora-checkpoint-reject.txt"
    } else {
        "/tmp/ora-checkpoint-reject.txt"
    });

    let parent = git.restore_paths(RestorePathsRequest {
        worktree: &worktree,
        commit_oid: &snapshot.commit_oid,
        paths: &["../x".to_string()],
    });
    assert!(parent.is_err(), "parent-dir path must be rejected");

    let absolute_result = git.restore_paths(RestorePathsRequest {
        worktree: &worktree,
        commit_oid: &snapshot.commit_oid,
        paths: &[absolute.to_string_lossy().into_owned()],
    });
    assert!(absolute_result.is_err(), "absolute path must be rejected");
    assert!(scaffold.repo_path().join("README.md").exists());
}

/// Snapshots a seeded repo, then adds/modifies/deletes files so restore tests share one fixture.
fn dirty_after_checkpoint(
    name: &str,
) -> (
    TestScaffold,
    Git<CliGitRunner>,
    gitlancer::WorktreeHandle,
    String,
) {
    let scaffold = TestScaffold::new(name).expect("create scaffold");
    scaffold
        .write_file(scaffold.repo_path(), "tracked.txt", "orig\n")
        .expect("write tracked");
    scaffold
        .write_file(scaffold.repo_path(), "gone.txt", "gone\n")
        .expect("write gone");
    scaffold
        .stage_all_and_commit("chore: seed restore fixture")
        .expect("commit seed");
    let (git, worktree) = runtime_worktree(&scaffold);
    let snapshot = git
        .snapshot_worktree(SnapshotWorktreeRequest {
            worktree: &worktree,
            name: "restore",
            message: "ora checkpoint: restore fixture",
        })
        .expect("snapshot before dirtying");
    scaffold
        .write_file(scaffold.repo_path(), "added.txt", "add\n")
        .expect("add file");
    scaffold
        .write_file(scaffold.repo_path(), "tracked.txt", "orig\nmod\n")
        .expect("modify file");
    std::fs::remove_file(scaffold.repo_path().join("gone.txt")).expect("delete file");
    (scaffold, git, worktree, snapshot.commit_oid)
}
