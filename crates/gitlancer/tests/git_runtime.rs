mod common;

use std::path::Path;

use common::TestScaffold;
use gitlancer::git::branch::{
    BranchDeletionMode, CreateBranchRequest, DeleteBranchRequest, ListBranchesRequest,
};
use gitlancer::git::commit::{AddRequest, CommitRequest};
use gitlancer::git::repository::ListWorktreesRequest;
use gitlancer::git::status::StatusRequest;
use gitlancer::git::worktree::{
    CreateWorktreeRequest, DeleteWorktreeRequest, FindWorktreeRequest, ResolveWorktreeRequest,
    WorktreeDeletionMode,
};
use gitlancer::{BranchName, CliGitRunner, Git, RepoRoot, WorktreeKind, WorktreeRoot};
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

    let created = git
        .create_branch(CreateBranchRequest {
            repository: &repository,
            branch_name: BranchName::new("feature/runtime"),
        })
        .expect("create branch");
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

/// Verifies linked worktree lifecycle APIs create and delete linked worktrees through typed runtime requests.
#[test]
fn runtime_creates_and_deletes_linked_worktrees() {
    let scaffold = TestScaffold::new("runtime-worktree-lifecycle").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, repository) = runtime_repository(&scaffold);
    let worktree_path = scaffold.linked_worktree_path("feature-tree");

    let created = git
        .create_worktree(CreateWorktreeRequest {
            repository: &repository,
            worktree_root: WorktreeRoot::new(&worktree_path),
            branch_name: BranchName::new("feature/runtime"),
        })
        .expect("create worktree");
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
