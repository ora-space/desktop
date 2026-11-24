//! Exercises the Git capability projection and guard through `WorkspaceApi`, against real
//! directories and real Git, because both are exactly what the probe observes.

use crate::Backend;
use crate::test_backend::backend_paths;
use ora_contracts::{
    CommitWorkspaceChangesRequest, CreateProjectRequest, CreateTaskRequest, EmptyErrorParams,
    GetWorkspaceDiffRequest, GetWorkspaceStatusRequest, ListWorkspacesRequest, PublicError,
    PushWorkspaceBranchRequest, StageWorkspaceChangesRequest, UnstageWorkspaceChangesRequest,
    Workspace, WorkspaceDiffScope, WorkspaceDirectory, WorkspaceGit, WorkspaceKind,
};
use ora_test_support::GitTestScaffold;
use pretty_assertions::{assert_eq, assert_ne};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Verifies a plain folder is reported as not Git, that every Git review operation refuses it
/// with one typed error, and that `git init` flips the capability on the next list.
#[test]
fn plain_folder_reports_not_git_and_refuses_git_operations_until_initialized() {
    let temporary = TempDir::new().expect("create temporary backend directory");
    let folder = temporary.path().join("plain");
    fs::create_dir_all(&folder).expect("create plain project folder");
    fs::write(folder.join("notes.txt"), "plain\n").expect("write plain file");
    let backend = open_backend(&temporary);
    let project_id = create_project(&backend, &folder);
    let workspace = main_workspace(&backend, &project_id);
    assert_eq!(
        workspace.directory,
        WorkspaceDirectory::Present {
            git: WorkspaceGit::NotRepository,
        }
    );

    let workspaces = backend.workspaces();
    let workspace_id = workspace.id.clone();
    let errors = vec![
        workspaces
            .get_diff(GetWorkspaceDiffRequest {
                workspace_id: workspace_id.clone(),
                scope: WorkspaceDiffScope::Unstaged,
            })
            .map(drop),
        // `Branch` would otherwise fail on the missing baseline; the folder must explain first.
        workspaces
            .get_diff(GetWorkspaceDiffRequest {
                workspace_id: workspace_id.clone(),
                scope: WorkspaceDiffScope::Branch,
            })
            .map(drop),
        workspaces
            .get_status(GetWorkspaceStatusRequest {
                workspace_id: workspace_id.clone(),
            })
            .map(drop),
        workspaces
            .stage_changes(StageWorkspaceChangesRequest {
                workspace_id: workspace_id.clone(),
                paths: vec![],
            })
            .map(drop),
        workspaces
            .unstage_changes(UnstageWorkspaceChangesRequest {
                workspace_id: workspace_id.clone(),
                paths: vec![],
            })
            .map(drop),
        workspaces
            .commit_changes(CommitWorkspaceChangesRequest {
                workspace_id: workspace_id.clone(),
                message: "plain".to_string(),
            })
            .map(drop),
        workspaces
            .push_branch(PushWorkspaceBranchRequest {
                workspace_id: workspace_id.clone(),
            })
            .map(drop),
    ]
    .into_iter()
    .map(|result| {
        result
            .expect_err("a plain folder refuses Git operations")
            .public_error()
            .clone()
    })
    .collect::<Vec<_>>();
    assert_eq!(
        errors,
        vec![PublicError::WorkspaceNotGitRepository(EmptyErrorParams {}); 7]
    );

    let initialized = Command::new("git")
        .arg("init")
        .arg("--quiet")
        .arg(&folder)
        .status()
        .expect("run git init");
    assert!(initialized.success());
    assert_eq!(
        main_workspace(&backend, &project_id).directory,
        WorkspaceDirectory::Present {
            git: WorkspaceGit::Repository,
        }
    );
    backend
        .workspaces()
        .get_status(GetWorkspaceStatusRequest { workspace_id })
        .expect("an initialized folder serves Git status");
}

/// Verifies an ordinary checkout, a project rooted at a linked worktree, and an isolated task
/// worktree all report `Repository`.
#[test]
fn checkouts_and_linked_worktrees_report_git() {
    let temporary = TempDir::new().expect("create temporary backend directory");
    let scaffold = seeded_scaffold("backend-workspace-git-checkouts");
    let linked = scaffold
        .create_linked_worktree("linked", "linked-branch")
        .expect("create linked worktree");
    let backend = open_backend(&temporary);
    let checkout_project = create_project(&backend, scaffold.repo_path());
    let linked_project = create_project(&backend, &linked);
    let task = backend
        .tasks()
        .create(CreateTaskRequest {
            project_id: checkout_project.clone(),
            title: "Isolated task".to_string(),
            base_branch: Some("main".to_string()),
        })
        .expect("create worktree task")
        .task;

    let repository = WorkspaceDirectory::Present {
        git: WorkspaceGit::Repository,
    };
    let listed = backend
        .workspaces()
        .list(ListWorkspacesRequest {})
        .expect("list workspaces")
        .workspaces;
    let directory_of = |id: &str| {
        listed
            .iter()
            .find(|workspace| workspace.id == id)
            .expect("workspace is listed")
            .directory
            .clone()
    };
    assert_eq!(
        vec![
            main_workspace(&backend, &checkout_project).directory,
            main_workspace(&backend, &linked_project).directory,
            directory_of(&task.workspace_id),
        ],
        vec![repository.clone(), repository.clone(), repository]
    );
}

/// Verifies a plain folder inside another checkout follows Git's upward discovery: it reports
/// `Repository`, and the review operations act on the enclosing checkout instead of refusing.
#[test]
fn folder_nested_in_a_checkout_reports_git_like_the_operations_behave() {
    let temporary = TempDir::new().expect("create temporary backend directory");
    let scaffold = seeded_scaffold("backend-workspace-git-nested");
    let nested = scaffold.repo_path().join("docs");
    fs::create_dir_all(&nested).expect("create nested folder");
    fs::write(nested.join("guide.md"), "nested\n").expect("write nested change");
    let backend = open_backend(&temporary);
    let project_id = create_project(&backend, &nested);
    let workspace = main_workspace(&backend, &project_id);

    assert_eq!(
        workspace.directory,
        WorkspaceDirectory::Present {
            git: WorkspaceGit::Repository,
        }
    );
    let diff = backend
        .workspaces()
        .get_diff(GetWorkspaceDiffRequest {
            workspace_id: workspace.id,
            scope: WorkspaceDiffScope::Unstaged,
        })
        .expect("a nested folder diffs its enclosing checkout");
    assert!(diff.patch.contains("docs/guide.md"));
}

/// Verifies a repository Git refuses for a reason other than "no repository here" (here a `.git`
/// link to a vanished repository) reports `Unknown`, and the review operations are not refused
/// as a plain folder, so nobody is told to `git init` a damaged checkout.
#[test]
fn damaged_repository_reports_unknown_rather_than_plain() {
    let temporary = TempDir::new().expect("create temporary backend directory");
    let folder = temporary.path().join("orphaned-worktree");
    fs::create_dir_all(&folder).expect("create orphaned worktree folder");
    fs::write(
        folder.join(".git"),
        format!("gitdir: {}\n", temporary.path().join("gone").display()),
    )
    .expect("write dangling gitdir link");
    let backend = open_backend(&temporary);
    let project_id = create_project(&backend, &folder);
    let workspace = main_workspace(&backend, &project_id);

    assert_eq!(
        workspace.directory,
        WorkspaceDirectory::Present {
            git: WorkspaceGit::Unknown,
        }
    );
    let error = backend
        .workspaces()
        .get_status(GetWorkspaceStatusRequest {
            workspace_id: workspace.id,
        })
        .expect_err("a damaged repository still fails")
        .public_error()
        .clone();
    assert_ne!(
        error,
        PublicError::WorkspaceNotGitRepository(EmptyErrorParams {})
    );
}

/// Verifies `.git` directories Git skips as invalid report `Unknown`, not `NotRepository`: Git
/// prints the plain-folder message for them, yet telling the user to `git init` would be wrong.
/// Covers a corrupt `HEAD`, an empty `.git`, and a damaged `.git` above a nested project folder.
#[test]
fn damaged_git_directories_report_unknown_rather_than_plain() {
    let temporary = TempDir::new().expect("create temporary backend directory");
    let corrupt = seeded_scaffold("backend-workspace-git-corrupt-head");
    fs::write(corrupt.repo_path().join(".git").join("HEAD"), "garbage\n").expect("corrupt HEAD");
    let empty = temporary.path().join("empty-git");
    fs::create_dir_all(empty.join(".git")).expect("create empty .git directory");
    let damaged_parent = temporary.path().join("damaged-parent");
    let nested = damaged_parent.join("nested");
    fs::create_dir_all(damaged_parent.join(".git")).expect("create damaged parent .git");
    fs::create_dir_all(&nested).expect("create nested folder");
    let backend = open_backend(&temporary);
    let projects = [
        create_project(&backend, corrupt.repo_path()),
        create_project(&backend, &empty),
        create_project(&backend, &nested),
    ];

    let unknown = WorkspaceDirectory::Present {
        git: WorkspaceGit::Unknown,
    };
    assert_eq!(
        projects
            .iter()
            .map(|project_id| main_workspace(&backend, project_id).directory)
            .collect::<Vec<_>>(),
        vec![unknown.clone(), unknown.clone(), unknown]
    );
}

/// Verifies a `.git` directory the user cannot read reports `Unknown`. Skipped when the test runs
/// with privileges that ignore file modes, since the folder is then readable after all.
#[cfg(unix)]
#[test]
fn unreadable_git_directory_reports_unknown() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = TempDir::new().expect("create temporary backend directory");
    let scaffold = seeded_scaffold("backend-workspace-git-unreadable");
    let git_directory = scaffold.repo_path().join(".git");
    fs::set_permissions(&git_directory, fs::Permissions::from_mode(0o000))
        .expect("make .git unreadable");
    let readable_anyway = fs::read_dir(&git_directory).is_ok();
    let directory = if readable_anyway {
        None
    } else {
        let backend = open_backend(&temporary);
        let project_id = create_project(&backend, scaffold.repo_path());
        Some(main_workspace(&backend, &project_id).directory)
    };
    // Restore before asserting so the fixture can always be removed.
    fs::set_permissions(&git_directory, fs::Permissions::from_mode(0o755))
        .expect("restore .git permissions");

    if let Some(directory) = directory {
        assert_eq!(
            directory,
            WorkspaceDirectory::Present {
                git: WorkspaceGit::Unknown,
            }
        );
    }
}

/// Verifies a project added through a symlink is checked along the real path Git walks: a
/// damaged `.git` above the link's target reports `Unknown`, not `NotRepository`.
#[cfg(unix)]
#[test]
fn symlinked_folder_under_a_damaged_repository_reports_unknown() {
    let temporary = TempDir::new().expect("create temporary backend directory");
    let repository = temporary.path().join("data").join("repo");
    let target = repository.join("docs");
    fs::create_dir_all(repository.join(".git")).expect("create damaged .git");
    fs::create_dir_all(&target).expect("create link target");
    let link = temporary.path().join("link");
    std::os::unix::fs::symlink(&target, &link).expect("create folder symlink");
    let backend = open_backend(&temporary);
    let project_id = create_project(&backend, &link);

    assert_eq!(
        main_workspace(&backend, &project_id).directory,
        WorkspaceDirectory::Present {
            git: WorkspaceGit::Unknown,
        }
    );
}

/// Verifies a missing folder reports `Missing` rather than any Git capability.
#[test]
fn missing_folder_reports_missing_without_git_capability() {
    let temporary = TempDir::new().expect("create temporary backend directory");
    let folder = temporary.path().join("gone");
    fs::create_dir_all(&folder).expect("create project folder");
    let backend = open_backend(&temporary);
    let project_id = create_project(&backend, &folder);
    fs::remove_dir_all(&folder).expect("delete project folder");

    assert_eq!(
        main_workspace(&backend, &project_id).directory,
        WorkspaceDirectory::Missing {
            path: folder.to_string_lossy().into_owned(),
        }
    );
}

/// Creates a repository fixture with one commit so branches and worktrees can be created.
fn seeded_scaffold(name: &str) -> GitTestScaffold {
    let scaffold = GitTestScaffold::new(name).expect("create Git test scaffold");
    scaffold
        .write_file(scaffold.repo_path(), "README.md", "ora backend test\n")
        .expect("write repository seed file");
    scaffold
        .stage_all_and_commit("initial")
        .expect("create repository seed commit");
    scaffold
}

/// Opens one isolated backend whose data and worktrees stay inside the test fixture.
fn open_backend(temporary: &TempDir) -> Backend {
    Backend::open(backend_paths(temporary.path(), temporary.path())).expect("open backend")
}

/// Persists a project rooted at `root` and returns its id.
fn create_project(backend: &Backend, root: &Path) -> String {
    backend
        .projects()
        .create(CreateProjectRequest {
            name: "Ora".to_string(),
            main_workspace_path: root.to_string_lossy().into_owned(),
        })
        .expect("create project")
        .project
        .id
}

/// Lists workspaces and returns the main workspace of one project.
fn main_workspace(backend: &Backend, project_id: &str) -> Workspace {
    backend
        .workspaces()
        .list(ListWorkspacesRequest {})
        .expect("list workspaces")
        .workspaces
        .into_iter()
        .find(|workspace| {
            workspace.project_id == project_id && workspace.kind == WorkspaceKind::Main
        })
        .expect("project has a main workspace")
}
