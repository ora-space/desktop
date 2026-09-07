use crate::{Backend, test_backend::backend_paths};
use ora_application::{Clock, SessionRepository};
use ora_contracts::*;
use ora_db::SqliteSessionRepository;
use ora_domain::{AgentRef, AuditFields, Session, SessionId, SessionStatus, WorkspaceId};
use ora_logging::with_trace_logging;
use ora_test_support::GitTestScaffold;
use pretty_assertions::assert_eq;
use std::fs;
use tempfile::TempDir;

/// Keeps bootstrap, repository setup, and asynchronous deletion under one scoped TRACE subscriber.
fn run_test(test: impl std::future::Future<Output = ()>) {
    with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(test);
    });
}

/// Both aggregate interfaces preserve active-resource refusal and atomic descendant retirement.
#[test]
fn running_session_blocks_both_cascades_and_stopped_session_is_retired_with_its_task() {
    run_test(async {
        let temporary = TempDir::new().expect("backend fixture");
        let git = GitTestScaffold::new("aggregate-session-refusal").expect("Git fixture");
        git.write_file(git.repo_path(), "README.md", "fixture\n")
            .expect("seed repository");
        git.stage_all_and_commit("initial").expect("initial commit");
        let backend =
            Backend::open(backend_paths(temporary.path(), temporary.path())).expect("backend");
        let projects = backend.projects();
        let tasks = backend.tasks();
        let project = projects
            .create(CreateProjectRequest {
                name: "Aggregate fixture".to_string(),
                main_workspace_path: git.repo_path().to_string_lossy().into_owned(),
            })
            .expect("project")
            .project;
        let task = tasks
            .create(CreateTaskRequest {
                project_id: project.id.clone(),
                title: "Protected task".to_string(),
                base_branch: Some("main".to_string()),
            })
            .expect("task")
            .task;
        let repository = SqliteSessionRepository::new(tasks.pool.clone());
        let session_id = SessionId::new("fixture-running-session");
        let now = crate::clock::SystemClock.now_timestamp_millis();
        let session = repository
            .create_session(Session::new(
                session_id.clone(),
                WorkspaceId::new(&task.workspace_id),
                AgentRef::parse("ora-space.fixture").expect("agent identity"),
                "provider-session",
                SessionStatus::Running,
                AuditFields::new(now, now, /*is_deleted*/ false),
            ))
            .expect("running session");
        for error in [
            tasks
                .delete(DeleteTaskRequest {
                    task_id: task.id.clone(),
                })
                .await
                .expect_err("task is protected"),
            projects
                .delete(DeleteProjectRequest {
                    project_id: project.id.clone(),
                })
                .await
                .expect_err("project is protected"),
        ] {
            assert_eq!(
                error.public_error(),
                &PublicError::ResourceInUse(EmptyErrorParams {})
            );
        }
        assert_eq!(
            tasks
                .get(GetTaskRequest {
                    task_id: task.id.clone()
                })
                .expect("task remains"),
            GetTaskResponse { task: task.clone() }
        );
        assert_eq!(
            projects
                .get(GetProjectRequest {
                    project_id: project.id.clone()
                })
                .expect("project remains"),
            GetProjectResponse {
                project: project.clone()
            }
        );
        assert_eq!(
            repository
                .find_session(&session_id)
                .expect("session remains"),
            Some(session)
        );

        repository
            .update_session_status(&session_id, SessionStatus::Stopped, now)
            .expect("release running session");
        let lease = tasks.git_cleanup.shared_worktree_use(&task.workspace_id);
        let checkout = temporary.path().join("worktrees").join(&task.workspace_id);
        assert_eq!(
            tasks
                .delete(DeleteTaskRequest {
                    task_id: task.id.clone()
                })
                .await
                .expect("delete task"),
            DeleteTaskResponse {
                task_id: task.id.clone(),
                workspace_id: task.workspace_id.clone()
            }
        );
        assert_eq!(
            repository
                .find_session(&session_id)
                .expect("retired session lookup"),
            None
        );
        assert!(
            checkout.is_dir(),
            "physical cleanup must wait for the existing use lease"
        );
        assert_eq!(
            tasks
                .get(GetTaskRequest { task_id: task.id })
                .expect_err("task retired")
                .public_error(),
            &PublicError::TaskNotFound(EmptyErrorParams {})
        );
        drop(lease);
        assert_eq!(
            projects
                .delete(DeleteProjectRequest {
                    project_id: project.id.clone()
                })
                .await
                .expect("delete project"),
            DeleteProjectResponse {
                project_id: project.id
            }
        );
    });
}

/// Verifies task deletion hides Ora records while deliberately preserving the Git worktree.
#[test]
fn deletes_existing_task_after_worktree_root_changes() {
    run_test(async {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let scaffold =
            GitTestScaffold::new("backend-task-deletion").expect("create Git test scaffold");
        scaffold
            .write_file(scaffold.repo_path(), "README.md", "ora backend test\n")
            .expect("write repository seed file");
        scaffold
            .stage_all_and_commit("initial")
            .expect("create repository seed commit");
        let repository_root = scaffold.repo_path().to_path_buf();
        let original_worktree_root = temporary.path().join("worktrees");
        let backend = Backend::open(backend_paths(temporary.path(), temporary.path()))
            .expect("open shared backend");
        let project = backend
            .projects()
            .create(CreateProjectRequest {
                name: "Ora".to_string(),
                main_workspace_path: repository_root.to_string_lossy().into_owned(),
            })
            .expect("create project")
            .project;
        let task = backend
            .tasks()
            .create(CreateTaskRequest {
                project_id: project.id,
                title: "Move configuration".to_string(),
                base_branch: Some("main".to_string()),
            })
            .expect("create task")
            .task;
        let original_worktree_path = original_worktree_root.join(&task.workspace_id);
        assert!(original_worktree_path.is_dir());

        let replacement_root = temporary.path().join("replacement-worktrees");
        fs::create_dir_all(&replacement_root).expect("create replacement worktree root");
        backend
            .workspaces()
            .set_worktree_root(replacement_root)
            .expect("replace worktree creation root");
        backend
            .tasks()
            .delete(DeleteTaskRequest {
                task_id: task.id.clone(),
            })
            .await
            .expect("delete task without Git mutation");

        assert!(original_worktree_path.exists());
        assert!(
            backend
                .tasks()
                .get(GetTaskRequest { task_id: task.id })
                .is_err()
        );
    });
}
