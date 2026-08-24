use std::ffi::OsStr;
use std::path::Path;

use gitlancer::git::base_branch::{
    ListWorktreeBasesRequest, ResolveWorktreeBaseCommitRequest, WorktreeBase,
};
use gitlancer::git::branch::{
    BranchDeletionMode, CreateBranchRequest, DeleteBranchRequest, ListBranchesRequest,
};
use gitlancer::git::commit::{AddRequest, CommitRequest};
use gitlancer::git::diff::DiffRequest;
use gitlancer::git::repository::ListWorktreesRequest;
use gitlancer::git::status::StatusRequest;
use gitlancer::git::worktree::{
    CreateWorktreeRequest, DeleteWorktreeRequest, FindWorktreeRequest,
    ResolveWorktreeByBranchRequest, ResolveWorktreeRequest, WorktreeDeletionMode,
};
use gitlancer::{BranchName, CliGitRunner, CommitId, Git, RepoRoot, WorktreeKind, WorktreeRoot};
use ora_test_support::GitTestScaffold as TestScaffold;
use ora_utils::path::canonicalize_longest_existing_prefix;
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

/// Compares filesystem identity so Windows short names and Git's long paths remain equivalent.
fn same_path(left: &Path, right: &Path) -> bool {
    canonicalize_longest_existing_prefix(left) == canonicalize_longest_existing_prefix(right)
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
    assert!(
        same_path(
            resolved_by_branch.worktree_root().as_path(),
            linked_path.as_path(),
        ),
        "branch metadata should resolve the authoritative linked worktree path"
    );
    assert!(
        same_path(found.worktree_root().as_path(), linked_path.as_path()),
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

/// Verifies local worktree bases remain usable without contacting a configured remote.
#[test]
fn runtime_lists_and_resolves_local_worktree_bases_without_fetching() {
    let scaffold = TestScaffold::new("runtime-local-worktree-bases").expect("create scaffold");
    seed_repository(&scaffold);
    let remote_path = scaffold.sandbox_root().join("missing-remote.git");
    scaffold
        .run_git([
            OsStr::new("remote"),
            OsStr::new("add"),
            OsStr::new("origin"),
            remote_path.as_os_str(),
        ])
        .expect("configure origin");
    let local_main_commit = scaffold
        .run_git(["rev-parse", "main"])
        .expect("resolve local main");

    let (git, repository) = runtime_repository(&scaffold);
    let bases = git
        .list_worktree_bases(ListWorktreeBasesRequest {
            repository: &repository,
        })
        .expect("list local worktree bases");
    let resolved = git
        .resolve_worktree_base_commit(ResolveWorktreeBaseCommitRequest {
            repository: &repository,
            reference_name: &BranchName::new("main"),
        })
        .expect("resolve local main");

    assert_eq!(
        bases.bases,
        vec![WorktreeBase::Local {
            branch_name: BranchName::new("main"),
        }]
    );
    assert_eq!(resolved.commit_id.as_str(), local_main_commit.trim());
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
            .any(|worktree| same_path(worktree.worktree_root().as_path(), &worktree_path)),
        "created worktrees should be visible through list_worktrees"
    );
    assert!(same_path(deleted.worktree_root.as_path(), &worktree_path));
    assert!(
        !worktrees_after_delete
            .worktrees
            .iter()
            .any(|worktree| same_path(worktree.worktree_root().as_path(), &worktree_path)),
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
            }) if same_path(&worktree, &linked_path) && same_path(&repo, right_repository.root().as_path())
        ),
        "cross-repository deletions should fail with WorktreeMismatch"
    );
}
