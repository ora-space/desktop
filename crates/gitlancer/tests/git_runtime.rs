mod common;

use std::path::Path;

use common::TestScaffold;
use gitlancer::git::base_branch::{
    ListWorktreeBasesRequest, ResolveWorktreeBaseCommitRequest, WorktreeBase,
};
use gitlancer::git::branch::{
    BranchDeletionMode, CheckoutBranchRequest, CreateBranchRequest, DeleteBranchRequest,
    ListBranchesRequest,
};
use gitlancer::git::commit::{AddRequest, CommitRequest, UnstageAllRequest, UnstageRequest};
use gitlancer::git::conflict::{ConflictSide, ResolveConflictRequest};
use gitlancer::git::diff::{CommitDiffRequest, DiffRequest};
use gitlancer::git::history::{GetCommitRequest, ListCommitsRequest, ListReferencesRequest};
use gitlancer::git::pull::FastForwardRequest;
use gitlancer::git::remote::{FetchAllRequest, ReadTrackingStatusRequest};
use gitlancer::git::repository::ListWorktreesRequest;
use gitlancer::git::status::StatusRequest;
use gitlancer::git::sync::{AbortSyncRequest, IntegrateRequest, SyncOperation, SyncResult};
use gitlancer::git::worktree::{
    CreateWorktreeRequest, DeleteWorktreeRequest, FindWorktreeRequest,
    ResolveWorktreeByBranchRequest, ResolveWorktreeRequest, WorktreeDeletionMode,
};
use gitlancer::{BranchName, CliGitRunner, CommitId, Git, RepoRoot, WorktreeKind, WorktreeRoot};
use pretty_assertions::assert_eq;

/// Creates an initial commit so linked worktrees can be created from a valid repository history.
fn seed_repository(scaffold: &TestScaffold) {
    scaffold
        .write_file(scaffold.repo_path(), "README.md", "seed repository\n")
        .expect("write seed file");
    scaffold
        .stage_all_and_commit("chore: seed repository")
        .expect("create initial commit");
}

/// Returns a typed runtime and repository handle for one scaffold so lifecycle tests can focus on behavior.
fn runtime_repository(scaffold: &TestScaffold) -> (Git<CliGitRunner>, gitlancer::Repository) {
    let git = Git::new(CliGitRunner);
    let repository = git
        .discover_repository(RepoRoot::new(scaffold.repo_path()))
        .expect("discover repository");

    (git, repository)
}

/// Verifies fixed-baseline diffs combine committed, staged, unstaged, and untracked changes.
#[test]
fn runtime_builds_complete_task_diff() {
    let scaffold = TestScaffold::new("runtime-builds-task-diff").expect("create scaffold");
    seed_repository(&scaffold);
    let base_commit_id = CommitId::new(
        scaffold
            .run_git(["rev-parse", "HEAD"])
            .expect("read base commit")
            .trim(),
    );
    scaffold
        .write_file(scaffold.repo_path(), "README.md", "committed change\n")
        .expect("write committed change");
    scaffold
        .stage_all_and_commit("feat: committed task change")
        .expect("commit task change");
    scaffold
        .write_file(scaffold.repo_path(), "staged.txt", "staged change\n")
        .expect("write staged change");
    scaffold
        .run_git(["add", "--", "staged.txt"])
        .expect("stage task change");
    let real_index_before = scaffold
        .run_git(["diff", "--cached", "--binary"])
        .expect("read real index before diff");
    scaffold
        .write_file(
            scaffold.repo_path(),
            "README.md",
            "committed change\nunstaged change\n",
        )
        .expect("write unstaged change");
    scaffold
        .write_file(scaffold.repo_path(), "untracked.txt", "untracked change\n")
        .expect("write untracked change");
    scaffold
        .write_file(scaffold.repo_path(), "empty.txt", "")
        .expect("write empty untracked file");
    scaffold
        .run_git(["config", "filter.guard.clean", "false"])
        .expect("configure failing clean filter");
    scaffold
        .run_git(["config", "filter.guard.required", "true"])
        .expect("require clean filter");
    scaffold
        .write_file(
            scaffold.repo_path(),
            ".gitattributes",
            "*.guard filter=guard\n",
        )
        .expect("write filter attributes");
    scaffold
        .write_file(
            scaffold.repo_path(),
            "untracked.guard",
            "filter must not run\n",
        )
        .expect("write filtered untracked file");
    std::fs::write(scaffold.repo_path().join("binary.bin"), b"\0binary\n")
        .expect("write untracked binary file");
    let (git, repository) = runtime_repository(&scaffold);
    let worktree = git
        .find_worktree(FindWorktreeRequest {
            repository: &repository,
            candidate_path: scaffold.repo_path(),
        })
        .expect("find main worktree");

    let response = git
        .diff(DiffRequest {
            worktree: &worktree,
            base_commit_id: &base_commit_id,
            scope: gitlancer::git::diff::DiffScope::Branch,
        })
        .expect("build task diff");

    assert_ne!(response.head_commit_id, base_commit_id);
    for expected_path in [
        "README.md",
        "empty.txt",
        "staged.txt",
        "untracked.txt",
        "untracked.guard",
        "binary.bin",
    ] {
        assert!(
            response
                .patch
                .contains(&format!("diff --git a/{expected_path} b/{expected_path}")),
            "patch should include {expected_path}"
        );
    }
    assert!(response.patch.contains("+unstaged change"));
    assert!(response.patch.contains("+untracked change"));
    let empty_file_patch = response
        .patch
        .split("diff --git ")
        .find(|section| section.starts_with("a/empty.txt b/empty.txt\n"))
        .expect("empty file should have its own patch section");
    assert!(empty_file_patch.contains("new file mode 100644"));
    assert!(empty_file_patch.contains("index 0000000..e69de29"));
    assert!(
        response
            .patch
            .contains("Binary files /dev/null and b/binary.bin differ")
    );
    assert!(!response.patch.contains("GIT binary patch"));
    let real_index_after = scaffold
        .run_git(["diff", "--cached", "--binary"])
        .expect("read real index after diff");
    assert_eq!(real_index_after, real_index_before);
}

/// Verifies the runtime can discover repositories, list worktrees, resolve linked worktrees, and enumerate branches.
#[test]
fn runtime_discovers_worktrees_and_branches() {
    let scaffold = TestScaffold::new("runtime-discovers-worktrees").expect("create scaffold");
    seed_repository(&scaffold);
    let linked_path = scaffold
        .create_linked_worktree("feature-tree", "feature/runtime")
        .expect("create linked worktree");

    let git = Git::new(CliGitRunner);
    let repository = git
        .discover_repository(RepoRoot::new(&linked_path))
        .expect("discover repository");
    let worktrees = git
        .list_worktrees(ListWorktreesRequest {
            repository: &repository,
        })
        .expect("list worktrees");
    let resolved = git
        .resolve_worktree(ResolveWorktreeRequest {
            repository: &repository,
            worktree_name: "feature-tree",
        })
        .expect("resolve linked worktree");
    let resolved_by_branch = git
        .resolve_worktree_by_branch(ResolveWorktreeByBranchRequest {
            repository: &repository,
            branch_name: "feature/runtime",
        })
        .expect("resolve linked worktree by branch");
    let nested_path = linked_path.join("src").join("nested.txt");
    let found = git
        .find_worktree(FindWorktreeRequest {
            repository: &repository,
            candidate_path: &nested_path,
        })
        .expect("find worktree");
    let branches = git
        .list_branches(ListBranchesRequest {
            repository: &repository,
        })
        .expect("list branches");

    assert_eq!(
        worktrees.worktrees.len(),
        2,
        "main and linked worktrees should be visible"
    );
    assert!(
        worktrees
            .worktrees
            .iter()
            .any(|worktree| matches!(worktree.kind(), WorktreeKind::Main)),
        "one worktree should be classified as the main checkout"
    );
    assert!(
        matches!(resolved.kind(), WorktreeKind::Linked { name } if name == "feature-tree"),
        "the resolved worktree should match the linked worktree name"
    );
    assert_eq!(
        resolved_by_branch.worktree_root().as_path(),
        linked_path.as_path(),
        "branch metadata should resolve the authoritative linked worktree path"
    );
    assert_eq!(
        found.worktree_root().as_path(),
        linked_path.as_path(),
        "nested paths should resolve back to the owning linked worktree"
    );
    assert!(
        branches
            .branches
            .iter()
            .any(|branch| branch.as_str() == "main"),
        "the seeded repository should keep its main branch"
    );
    assert!(
        branches
            .branches
            .iter()
            .any(|branch| branch.as_str() == "feature/runtime"),
        "the linked worktree branch should be listed as a local branch"
    );
}

/// Verifies status, add, and commit flows return typed results when operating inside a linked worktree.
#[test]
fn runtime_reports_status_and_commit_metadata() {
    let scaffold = TestScaffold::new("runtime-status-and-commit").expect("create scaffold");
    seed_repository(&scaffold);
    let linked_path = scaffold
        .create_linked_worktree("feature-tree", "feature/runtime")
        .expect("create linked worktree");
    scaffold
        .write_file(&linked_path, "linked.txt", "linked worktree change\n")
        .expect("write linked file");

    let git = Git::new(CliGitRunner);
    let repository = git
        .discover_repository(RepoRoot::new(scaffold.repo_path()))
        .expect("discover repository");
    let worktree = git
        .resolve_worktree(ResolveWorktreeRequest {
            repository: &repository,
            worktree_name: "feature-tree",
        })
        .expect("resolve linked worktree");
    let status_before_add = git
        .status(StatusRequest {
            worktree: &worktree,
        })
        .expect("read worktree status before add");
    let add_result = git
        .add(AddRequest {
            worktree: &worktree,
            paths: vec![
                worktree
                    .resolve_repo_relative_path(Path::new("linked.txt"))
                    .expect("resolve linked file path"),
            ],
        })
        .expect("stage linked file");
    git.unstage(UnstageRequest {
        worktree: &worktree,
        paths: vec![
            worktree
                .resolve_repo_relative_path(Path::new("linked.txt"))
                .expect("resolve linked file path for unstage"),
        ],
    })
    .expect("unstage linked file");
    git.add(AddRequest {
        worktree: &worktree,
        paths: vec![
            worktree
                .resolve_repo_relative_path(Path::new("linked.txt"))
                .expect("resolve linked file path for restage"),
        ],
    })
    .expect("restage linked file");
    let commit_result = git
        .commit(CommitRequest {
            worktree: &worktree,
            message: "feat: commit linked worktree change",
            allow_empty: false,
        })
        .expect("commit linked worktree change");

    assert!(
        status_before_add
            .entries
            .iter()
            .any(|entry| entry.raw.contains("linked.txt")),
        "status should include the untracked linked file before staging"
    );
    assert_eq!(
        add_result.staged_paths[0].as_path(),
        Path::new("linked.txt"),
        "the staged path should remain repo-relative"
    );
    assert_eq!(
        commit_result.summary, "feat: commit linked worktree change",
        "commit should return the latest summary"
    );
    assert_eq!(
        commit_result.commit_id.as_str().len(),
        40,
        "commit should return a full object ID"
    );
}

/// Verifies the all-path unstage operation preserves worktree content while clearing the index.
#[test]
fn runtime_unstages_all_paths_without_discarding_files() {
    let scaffold = TestScaffold::new("runtime-unstage-all").expect("create scaffold");
    seed_repository(&scaffold);
    scaffold
        .write_file(scaffold.repo_path(), "README.md", "changed README\n")
        .expect("write tracked change");
    scaffold
        .write_file(scaffold.repo_path(), "notes.txt", "untracked note\n")
        .expect("write untracked change");

    let (git, repository) = runtime_repository(&scaffold);
    let worktree = git
        .find_worktree(FindWorktreeRequest {
            repository: &repository,
            candidate_path: scaffold.repo_path(),
        })
        .expect("find main worktree");
    git.stage_all(gitlancer::git::commit::StageAllRequest {
        worktree: &worktree,
    })
    .expect("stage all changes");
    assert!(
        scaffold
            .run_git(["diff", "--cached", "--name-only"])
            .expect("read staged paths")
            .contains("README.md")
    );

    git.unstage_all(UnstageAllRequest {
        worktree: &worktree,
    })
    .expect("unstage all changes");

    assert!(
        scaffold
            .run_git(["diff", "--cached", "--name-only"])
            .expect("read cleared staged paths")
            .trim()
            .is_empty()
    );
    assert_eq!(
        std::fs::read_to_string(scaffold.repo_path().join("README.md")).expect("read tracked file"),
        "changed README\n"
    );
    assert_eq!(
        std::fs::read_to_string(scaffold.repo_path().join("notes.txt"))
            .expect("read untracked file"),
        "untracked note\n"
    );
}

/// Verifies local remote synchronization reports upstream distance and preserves diverged refs.
#[test]
fn runtime_fetches_and_reads_remote_tracking_distance() {
    let scaffold = TestScaffold::new("runtime-remote-tracking").expect("create scaffold");
    seed_repository(&scaffold);
    scaffold
        .run_git_in(scaffold.sandbox_root(), ["init", "--bare", "remote.git"])
        .expect("create bare remote");
    let remote_path = scaffold
        .sandbox_root()
        .join("remote.git")
        .to_string_lossy()
        .into_owned();
    scaffold
        .run_git(["remote", "add", "origin", remote_path.as_str()])
        .expect("configure origin");

    let (git, repository) = runtime_repository(&scaffold);
    let worktree = git
        .find_worktree(FindWorktreeRequest {
            repository: &repository,
            candidate_path: scaffold.repo_path(),
        })
        .expect("find main worktree");
    let pushed = git.push_branch(&worktree).expect("push initial branch");
    assert_eq!(pushed.branch_name, "main");
    assert_eq!(pushed.remote_name, "origin");
    assert_eq!(
        git.read_tracking_status(ReadTrackingStatusRequest {
            repository: &repository,
        })
        .expect("read synchronized tracking status"),
        gitlancer::git::remote::ReadTrackingStatusResponse {
            upstream: Some("origin/main".to_string()),
            ahead: 0,
            behind: 0,
        }
    );

    scaffold
        .write_file(scaffold.repo_path(), "local.txt", "local commit\n")
        .expect("write local commit");
    scaffold
        .stage_all_and_commit("feat: local remote change")
        .expect("commit local change");
    assert_eq!(
        git.read_tracking_status(ReadTrackingStatusRequest {
            repository: &repository,
        })
        .expect("read ahead tracking status")
        .ahead,
        1
    );

    let remote_clone = scaffold.sandbox_root().join("remote-clone");
    scaffold
        .run_git_in(
            scaffold.sandbox_root(),
            [
                "clone",
                "--branch",
                "main",
                remote_path.as_str(),
                "remote-clone",
            ],
        )
        .expect("clone remote repository");
    scaffold
        .run_git_in(&remote_clone, ["config", "user.name", "Remote Tests"])
        .expect("configure remote clone name");
    scaffold
        .run_git_in(
            &remote_clone,
            ["config", "user.email", "remote-tests@example.com"],
        )
        .expect("configure remote clone email");
    scaffold
        .write_file(&remote_clone, "remote.txt", "remote commit\n")
        .expect("write remote commit");
    scaffold
        .run_git_in(&remote_clone, ["add", "remote.txt"])
        .expect("stage remote commit");
    scaffold
        .run_git_in(&remote_clone, ["commit", "-m", "feat: remote change"])
        .expect("commit remote change");
    scaffold
        .run_git_in(&remote_clone, ["push", "origin", "main"])
        .expect("push remote change");

    git.fetch_all(FetchAllRequest {
        repository: &repository,
    })
    .expect("fetch remote refs");
    assert_eq!(
        git.read_tracking_status(ReadTrackingStatusRequest {
            repository: &repository,
        })
        .expect("read diverged tracking status"),
        gitlancer::git::remote::ReadTrackingStatusResponse {
            upstream: Some("origin/main".to_string()),
            ahead: 1,
            behind: 1,
        }
    );
}

/// Verifies a clean branch advances to a fetched upstream without creating a merge commit.
#[test]
fn runtime_fast_forwards_current_branch_from_upstream() {
    let scaffold = TestScaffold::new("runtime-fast-forward").expect("create scaffold");
    seed_repository(&scaffold);
    scaffold
        .run_git_in(scaffold.sandbox_root(), ["init", "--bare", "remote.git"])
        .expect("create bare remote");
    let remote_path = scaffold
        .sandbox_root()
        .join("remote.git")
        .to_string_lossy()
        .into_owned();
    scaffold
        .run_git(["remote", "add", "origin", remote_path.as_str()])
        .expect("configure origin");

    let (git, repository) = runtime_repository(&scaffold);
    let worktree = git
        .find_worktree(FindWorktreeRequest {
            repository: &repository,
            candidate_path: scaffold.repo_path(),
        })
        .expect("find main worktree");
    git.push_branch(&worktree).expect("push initial branch");

    let remote_clone = scaffold.sandbox_root().join("remote-clone");
    scaffold
        .run_git_in(
            scaffold.sandbox_root(),
            [
                "clone",
                "--branch",
                "main",
                remote_path.as_str(),
                "remote-clone",
            ],
        )
        .expect("clone remote repository");
    scaffold
        .run_git_in(&remote_clone, ["config", "user.name", "Remote Tests"])
        .expect("configure remote clone name");
    scaffold
        .run_git_in(
            &remote_clone,
            ["config", "user.email", "remote-tests@example.com"],
        )
        .expect("configure remote clone email");
    scaffold
        .write_file(&remote_clone, "remote.txt", "remote commit\n")
        .expect("write remote commit");
    scaffold
        .run_git_in(&remote_clone, ["add", "remote.txt"])
        .expect("stage remote commit");
    scaffold
        .run_git_in(&remote_clone, ["commit", "-m", "feat: remote fast-forward"])
        .expect("commit remote change");
    scaffold
        .run_git_in(&remote_clone, ["push", "origin", "main"])
        .expect("push remote change");

    git.fetch_all(FetchAllRequest {
        repository: &repository,
    })
    .expect("fetch remote refs");
    git.fast_forward(FastForwardRequest {
        worktree: &worktree,
        upstream: "origin/main",
    })
    .expect("fast-forward local branch");

    assert_eq!(
        std::fs::read_to_string(scaffold.repo_path().join("remote.txt"))
            .expect("read fast-forwarded file"),
        "remote commit\n"
    );
    assert_eq!(
        git.read_tracking_status(ReadTrackingStatusRequest {
            repository: &repository,
        })
        .expect("read synchronized tracking status"),
        gitlancer::git::remote::ReadTrackingStatusResponse {
            upstream: Some("origin/main".to_string()),
            ahead: 0,
            behind: 0,
        }
    );
}

/// Verifies a real merge conflict is surfaced as an active operation and can be safely aborted.
#[test]
fn runtime_surfaces_and_aborts_merge_conflicts() {
    let scaffold = TestScaffold::new("runtime-merge-conflict").expect("create scaffold");
    seed_repository(&scaffold);
    scaffold
        .run_git_in(scaffold.sandbox_root(), ["init", "--bare", "remote.git"])
        .expect("create bare remote");
    let remote_path = scaffold
        .sandbox_root()
        .join("remote.git")
        .to_string_lossy()
        .into_owned();
    scaffold
        .run_git(["remote", "add", "origin", remote_path.as_str()])
        .expect("configure origin");

    let (git, repository) = runtime_repository(&scaffold);
    let worktree = git
        .find_worktree(FindWorktreeRequest {
            repository: &repository,
            candidate_path: scaffold.repo_path(),
        })
        .expect("find main worktree");
    git.push_branch(&worktree).expect("push initial branch");

    scaffold
        .write_file(scaffold.repo_path(), "README.md", "local change\n")
        .expect("write local conflicting change");
    scaffold
        .stage_all_and_commit("feat: local conflicting change")
        .expect("commit local conflicting change");

    let remote_clone = scaffold.sandbox_root().join("remote-clone");
    scaffold
        .run_git_in(
            scaffold.sandbox_root(),
            [
                "clone",
                "--branch",
                "main",
                remote_path.as_str(),
                "remote-clone",
            ],
        )
        .expect("clone remote repository");
    scaffold
        .run_git_in(&remote_clone, ["config", "user.name", "Remote Tests"])
        .expect("configure remote clone name");
    scaffold
        .run_git_in(
            &remote_clone,
            ["config", "user.email", "remote-tests@example.com"],
        )
        .expect("configure remote clone email");
    scaffold
        .write_file(&remote_clone, "README.md", "remote change\n")
        .expect("write remote conflicting change");
    scaffold
        .run_git_in(&remote_clone, ["add", "README.md"])
        .expect("stage remote conflicting change");
    scaffold
        .run_git_in(
            &remote_clone,
            ["commit", "-m", "feat: remote conflicting change"],
        )
        .expect("commit remote conflicting change");
    scaffold
        .run_git_in(&remote_clone, ["push", "origin", "main"])
        .expect("push remote conflicting change");

    git.fetch_all(FetchAllRequest {
        repository: &repository,
    })
    .expect("fetch remote refs");
    assert_eq!(
        git.integrate(IntegrateRequest {
            repository: &repository,
            worktree: &worktree,
            upstream: "origin/main",
            operation: SyncOperation::Merge,
        })
        .expect("surface merge conflict"),
        SyncResult::Conflicted
    );
    assert_eq!(
        git.read_sync_operation(gitlancer::git::sync::ReadSyncOperationRequest {
            repository: &repository,
        })
        .expect("read active merge")
        .expect("merge should remain active"),
        SyncOperation::Merge
    );
    assert!(
        git.status(StatusRequest {
            worktree: &worktree,
        })
        .expect("read conflicted status")
        .entries
        .iter()
        .any(|entry| entry.raw.starts_with("u ")),
        "the merge should expose an unmerged status entry"
    );
    let conflict_path = worktree
        .resolve_repo_relative_path(Path::new("README.md"))
        .expect("resolve conflict path");
    git.resolve_conflict(ResolveConflictRequest {
        worktree: &worktree,
        path: &conflict_path,
        side: ConflictSide::Ours,
    })
    .expect("select local conflict side");
    assert_eq!(
        std::fs::read_to_string(scaffold.repo_path().join("README.md"))
            .expect("read selected local file"),
        "local change\n"
    );
    assert!(
        !git.status(StatusRequest {
            worktree: &worktree,
        })
        .expect("read resolved status")
        .entries
        .iter()
        .any(|entry| entry.raw.starts_with("u "))
    );

    git.abort_sync(AbortSyncRequest {
        repository: &repository,
        worktree: &worktree,
        operation: SyncOperation::Merge,
    })
    .expect("abort merge conflict");
    assert_eq!(
        git.read_sync_operation(gitlancer::git::sync::ReadSyncOperationRequest {
            repository: &repository,
        })
        .expect("read cleared merge state"),
        None
    );
    assert_eq!(
        std::fs::read_to_string(scaffold.repo_path().join("README.md"))
            .expect("read restored local file"),
        "local change\n"
    );
}

/// Verifies history queries preserve commit topology, ref labels, HEAD state, and changed paths.
#[test]
fn runtime_reads_repository_history_and_commit_details() {
    let scaffold = TestScaffold::new("runtime-repository-history").expect("create scaffold");
    seed_repository(&scaffold);
    scaffold
        .write_file(scaffold.repo_path(), "history.txt", "history change\n")
        .expect("write history file");
    scaffold
        .stage_all_and_commit("feat: add history file")
        .expect("create history commit");

    let (git, repository) = runtime_repository(&scaffold);
    let history = git
        .list_commits(ListCommitsRequest {
            repository: &repository,
            limit: 10,
        })
        .expect("read repository history");
    let references = git
        .list_references(ListReferencesRequest {
            repository: &repository,
        })
        .expect("read repository refs");
    let head = git.read_head(&repository).expect("read repository head");
    let selected_commit_id = history.commits[0].id.clone();
    let details = git
        .get_commit(GetCommitRequest {
            repository: &repository,
            commit_id: &selected_commit_id,
        })
        .expect("read commit details");
    let commit_diff = git
        .diff_commit(CommitDiffRequest {
            repository: &repository,
            commit_id: &selected_commit_id,
            parent_commit_id: details.commit.summary.parents.first(),
            path: None,
        })
        .expect("read commit diff");

    assert_eq!(history.commits.len(), 2);
    assert_eq!(history.commits[0].parents.len(), 1);
    assert!(
        references
            .references
            .iter()
            .any(|reference| reference.name == "main")
    );
    assert_eq!(
        head.commit_id.as_ref(),
        Some(&selected_commit_id),
        "HEAD should point at the newest history row"
    );
    assert_eq!(
        head.branch_name.as_ref().map(|branch| branch.as_str()),
        Some("main")
    );
    assert_eq!(details.commit.summary.id, selected_commit_id);
    assert_eq!(
        details.commit.files[0].path, "history.txt",
        "commit details should expose the changed path"
    );
    assert!(
        commit_diff
            .patch
            .contains("diff --git a/history.txt b/history.txt")
    );
    assert!(commit_diff.patch.contains("+history change"));
}

/// Verifies repo-relative path resolution rejects traversal attempts that escape the worktree root.
#[test]
fn worktree_rejects_paths_outside_the_checkout() {
    let scaffold = TestScaffold::new("runtime-rejects-outside-paths").expect("create scaffold");
    seed_repository(&scaffold);
    let linked_path = scaffold
        .create_linked_worktree("feature-tree", "feature/runtime")
        .expect("create linked worktree");

    let git = Git::new(CliGitRunner);
    let repository = git
        .discover_repository(RepoRoot::new(&linked_path))
        .expect("discover repository");
    let worktree = git
        .resolve_worktree(ResolveWorktreeRequest {
            repository: &repository,
            worktree_name: "feature-tree",
        })
        .expect("resolve linked worktree");
    let outside = scaffold.sandbox_root().join("outside.txt");

    let error = worktree
        .resolve_repo_relative_path(&outside)
        .expect_err("outside paths must be rejected");

    assert!(
        matches!(error, gitlancer::DomainError::PathOutsideWorktree { .. }),
        "paths outside the worktree should fail with PathOutsideWorktree"
    );
}

/// Verifies branch lifecycle APIs create and delete local branches through typed repository requests.
#[test]
fn runtime_creates_and_deletes_local_branches() {
    let scaffold = TestScaffold::new("runtime-branch-lifecycle").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, repository) = runtime_repository(&scaffold);
    let base_commit = scaffold
        .run_git(["rev-parse", "HEAD"])
        .expect("resolve base commit");
    scaffold
        .write_file(scaffold.repo_path(), "later.txt", "later commit\n")
        .expect("write later commit");
    scaffold
        .stage_all_and_commit("later commit")
        .expect("create later commit");

    let created = git
        .create_branch(CreateBranchRequest {
            repository: &repository,
            branch_name: BranchName::new("feature/runtime"),
            commit_id: CommitId::new(base_commit.trim()),
        })
        .expect("create branch");
    let created_commit = scaffold
        .run_git(["rev-parse", "feature/runtime"])
        .expect("resolve created branch");
    let branches_after_create = git
        .list_branches(ListBranchesRequest {
            repository: &repository,
        })
        .expect("list branches after create");
    let deleted = git
        .delete_branch(DeleteBranchRequest {
            repository: &repository,
            branch_name: BranchName::new("feature/runtime"),
            mode: BranchDeletionMode::Checked,
        })
        .expect("delete branch");
    let branches_after_delete = git
        .list_branches(ListBranchesRequest {
            repository: &repository,
        })
        .expect("list branches after delete");

    assert_eq!(created.branch, BranchName::new("feature/runtime"));
    assert_eq!(created_commit.trim(), base_commit.trim());
    assert!(
        branches_after_create
            .branches
            .iter()
            .any(|branch| branch.as_str() == "feature/runtime"),
        "created branches should be visible through list_branches"
    );
    assert_eq!(deleted.branch, BranchName::new("feature/runtime"));
    assert!(
        !branches_after_delete
            .branches
            .iter()
            .any(|branch| branch.as_str() == "feature/runtime"),
        "deleted branches should no longer be visible through list_branches"
    );
}

/// Verifies a local branch can be selected in the main worktree without creating another ref.
#[test]
fn runtime_checks_out_an_existing_local_branch() {
    let scaffold = TestScaffold::new("runtime-checkout-branch").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, repository) = runtime_repository(&scaffold);
    let base_commit = scaffold
        .run_git(["rev-parse", "HEAD"])
        .expect("resolve base commit");

    git.create_branch(CreateBranchRequest {
        repository: &repository,
        branch_name: BranchName::new("feature/runtime"),
        commit_id: CommitId::new(base_commit.trim()),
    })
    .expect("create checkout target branch");
    let worktree = git
        .find_worktree(FindWorktreeRequest {
            repository: &repository,
            candidate_path: scaffold.repo_path(),
        })
        .expect("find main worktree");

    let checked_out = git
        .checkout_branch(CheckoutBranchRequest {
            worktree: &worktree,
            branch_name: &BranchName::new("feature/runtime"),
        })
        .expect("checkout feature branch");
    let current_branch = scaffold
        .run_git(["branch", "--show-current"])
        .expect("read selected branch");

    assert_eq!(checked_out.branch, BranchName::new("feature/runtime"));
    assert_eq!(current_branch.trim(), "feature/runtime");

    git.checkout_branch(CheckoutBranchRequest {
        worktree: &worktree,
        branch_name: &BranchName::new("main"),
    })
    .expect("restore main branch");
}

/// Verifies fetched remote refs replace stale local duplicates and expose remote-only branches.
#[test]
fn runtime_lists_and_resolves_fresh_remote_worktree_bases() {
    let scaffold = TestScaffold::new("runtime-remote-worktree-bases").expect("create scaffold");
    seed_repository(&scaffold);
    let remote_path = scaffold.sandbox_root().join("remote.git");
    let remote_path_arg = remote_path.to_string_lossy().into_owned();
    scaffold
        .run_git_in(
            scaffold.sandbox_root(),
            ["init", "--bare", "--initial-branch=main", &remote_path_arg],
        )
        .expect("create bare remote");
    scaffold
        .run_git(["remote", "add", "origin", &remote_path_arg])
        .expect("configure origin");
    scaffold
        .run_git(["push", "-u", "origin", "main"])
        .expect("push initial main");

    let stale_main_commit = scaffold
        .run_git(["rev-parse", "main"])
        .expect("resolve stale main");
    scaffold
        .run_git(["switch", "-c", "frontend"])
        .expect("create frontend branch");
    scaffold
        .write_file(scaffold.repo_path(), "frontend.txt", "remote-only branch\n")
        .expect("write frontend fixture");
    scaffold
        .stage_all_and_commit("add frontend branch")
        .expect("commit frontend branch");
    scaffold
        .run_git(["push", "origin", "frontend"])
        .expect("push frontend branch");
    scaffold
        .run_git(["switch", "main"])
        .expect("switch back to main");
    scaffold
        .run_git(["branch", "-D", "frontend"])
        .expect("delete local frontend branch");
    scaffold
        .write_file(scaffold.repo_path(), "latest.txt", "latest remote main\n")
        .expect("write latest main fixture");
    scaffold
        .stage_all_and_commit("advance remote main")
        .expect("commit latest main");
    let fresh_main_commit = scaffold
        .run_git(["rev-parse", "main"])
        .expect("resolve fresh main");
    scaffold
        .run_git(["push", "origin", "main"])
        .expect("push latest main");
    scaffold
        .run_git(["reset", "--hard", stale_main_commit.trim()])
        .expect("restore stale local main");

    let (git, repository) = runtime_repository(&scaffold);
    let bases = git
        .list_worktree_bases(ListWorktreeBasesRequest {
            repository: &repository,
        })
        .expect("list refreshed worktree bases");
    let resolved = git
        .resolve_worktree_base_commit(ResolveWorktreeBaseCommitRequest {
            repository: &repository,
            reference_name: &BranchName::new("origin/main"),
        })
        .expect("resolve refreshed remote main");

    assert_eq!(
        bases.bases,
        vec![
            WorktreeBase::Remote {
                remote_name: "origin".to_string(),
                branch_name: BranchName::new("frontend"),
            },
            WorktreeBase::Remote {
                remote_name: "origin".to_string(),
                branch_name: BranchName::new("main"),
            },
        ]
    );
    assert_eq!(resolved.commit_id.as_str(), fresh_main_commit.trim());
    assert_ne!(resolved.commit_id.as_str(), stale_main_commit.trim());
}

/// Verifies linked worktree lifecycle APIs create and delete linked worktrees through typed runtime requests.
#[test]
fn runtime_creates_and_deletes_linked_worktrees() {
    let scaffold = TestScaffold::new("runtime-worktree-lifecycle").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, repository) = runtime_repository(&scaffold);
    let worktree_path = scaffold.linked_worktree_path("feature-tree");
    let base_commit = scaffold
        .run_git(["rev-parse", "HEAD"])
        .expect("resolve base commit");
    scaffold
        .write_file(scaffold.repo_path(), "later.txt", "later commit\n")
        .expect("write later commit");
    scaffold
        .stage_all_and_commit("later commit")
        .expect("create later commit");

    let created = git
        .create_worktree(CreateWorktreeRequest {
            repository: &repository,
            worktree_root: WorktreeRoot::new(&worktree_path),
            branch_name: BranchName::new("feature/runtime"),
            base_commit_id: CommitId::new(base_commit.trim()),
        })
        .expect("create worktree");
    let worktree_commit = scaffold
        .run_git_in(&worktree_path, ["rev-parse", "HEAD"])
        .expect("resolve worktree commit");
    let worktrees_after_create = git
        .list_worktrees(ListWorktreesRequest {
            repository: &repository,
        })
        .expect("list worktrees after create");
    let deleted = git
        .delete_worktree(DeleteWorktreeRequest {
            repository: &repository,
            worktree: &created.worktree,
            mode: WorktreeDeletionMode::Checked,
        })
        .expect("delete linked worktree");
    let worktrees_after_delete = git
        .list_worktrees(ListWorktreesRequest {
            repository: &repository,
        })
        .expect("list worktrees after delete");

    assert_eq!(worktree_commit.trim(), base_commit.trim());
    assert!(
        matches!(created.worktree.kind(), WorktreeKind::Linked { name } if name == "feature-tree"),
        "created worktrees should come back as linked worktrees"
    );
    assert!(
        worktrees_after_create
            .worktrees
            .iter()
            .any(|worktree| worktree.worktree_root().as_path() == worktree_path.as_path()),
        "created worktrees should be visible through list_worktrees"
    );
    assert_eq!(deleted.worktree_root, WorktreeRoot::new(&worktree_path));
    assert!(
        !worktrees_after_delete
            .worktrees
            .iter()
            .any(|worktree| worktree.worktree_root().as_path() == worktree_path.as_path()),
        "deleted worktrees should no longer be visible through list_worktrees"
    );
}

/// Verifies main-worktree deletion is rejected before Git attempts a destructive worktree removal.
#[test]
fn runtime_rejects_main_worktree_deletion() {
    let scaffold =
        TestScaffold::new("runtime-rejects-main-worktree-delete").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, repository) = runtime_repository(&scaffold);
    let worktrees = git
        .list_worktrees(ListWorktreesRequest {
            repository: &repository,
        })
        .expect("list worktrees");
    let main_worktree = worktrees
        .worktrees
        .into_iter()
        .find(|worktree| matches!(worktree.kind(), WorktreeKind::Main))
        .expect("main worktree");

    let error = git
        .delete_worktree(DeleteWorktreeRequest {
            repository: &repository,
            worktree: &main_worktree,
            mode: WorktreeDeletionMode::Checked,
        })
        .expect_err("main worktree deletion should be rejected");

    assert!(
        matches!(
            error,
            gitlancer::GitlancerError::Domain(
                gitlancer::DomainError::MainWorktreeDeletionUnsupported(repo)
            ) if repo == repository.root().as_path()
        ),
        "main worktree deletion should fail with MainWorktreeDeletionUnsupported"
    );
}

/// Verifies worktree deletion rejects linked worktrees that do not belong to the supplied repository.
#[test]
fn runtime_rejects_cross_repository_worktree_deletion() {
    let left = TestScaffold::new("runtime-worktree-mismatch-left").expect("create left scaffold");
    let right =
        TestScaffold::new("runtime-worktree-mismatch-right").expect("create right scaffold");
    seed_repository(&left);
    seed_repository(&right);

    let (left_git, left_repository) = runtime_repository(&left);
    let (_, right_repository) = runtime_repository(&right);
    let linked_path = left
        .create_linked_worktree("feature-tree", "feature/runtime")
        .expect("create linked worktree");
    let linked_worktree = left_git
        .resolve_worktree(ResolveWorktreeRequest {
            repository: &left_repository,
            worktree_name: "feature-tree",
        })
        .expect("resolve linked worktree");

    let error = left_git
        .delete_worktree(DeleteWorktreeRequest {
            repository: &right_repository,
            worktree: &linked_worktree,
            mode: WorktreeDeletionMode::Checked,
        })
        .expect_err("cross-repository worktree deletion should be rejected");

    assert!(
        matches!(
            error,
            gitlancer::GitlancerError::Domain(gitlancer::DomainError::WorktreeMismatch {
                worktree,
                repo,
            }) if worktree == linked_path && repo == right_repository.root().as_path()
        ),
        "cross-repository deletions should fail with WorktreeMismatch"
    );
}
