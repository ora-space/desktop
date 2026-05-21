use crate::app_state::AppState;
use crate::handlers::{health, project_work_contexts, projects, sessions, tasks};
use axum::Router;
use axum::routing::{get, post};
use ora_contracts::{
    PROJECT_PATH, PROJECT_WORK_CONTEXT_OPEN_PATH, PROJECT_WORK_CONTEXT_RENEW_PATH, PROJECTS_PATH,
    SESSION_PATH, SESSION_TERMINAL_PATH, SESSIONS_PATH, TASK_PATH, TASKS_PATH,
};

/// Builds the top-level router for health checks and the persisted CRUD routes.
pub fn build_router(app_state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(health::liveness))
        .route("/health/ready", get(health::readiness))
        .route(
            PROJECTS_PATH,
            post(projects::create_project).get(projects::list_projects),
        )
        .route(
            PROJECT_PATH,
            get(projects::get_project)
                .put(projects::update_project)
                .delete(projects::delete_project),
        )
        .route(
            PROJECT_WORK_CONTEXT_OPEN_PATH,
            post(project_work_contexts::open_project_work_context),
        )
        .route(
            PROJECT_WORK_CONTEXT_RENEW_PATH,
            post(project_work_contexts::renew_project_work_context),
        )
        .route(TASKS_PATH, post(tasks::create_task).get(tasks::list_tasks))
        .route(
            TASK_PATH,
            get(tasks::get_task)
                .put(tasks::update_task)
                .delete(tasks::delete_task),
        )
        .route(
            SESSIONS_PATH,
            post(sessions::create_session).get(sessions::list_sessions),
        )
        .route(
            SESSION_PATH,
            get(sessions::get_session)
                .put(sessions::update_session)
                .delete(sessions::delete_session),
        )
        .route(
            SESSION_TERMINAL_PATH,
            get(sessions::attach_terminal_session),
        )
        .with_state(app_state)
}

#[cfg(test)]
mod tests {
    use super::build_router;
    use crate::app_state::AppState;
    use crate::bootstrap::build_app_state_for_database;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use futures_util::StreamExt;
    use ora_application::{ProjectWorkContextRepository, WorktreeRepository};
    use ora_contracts::TerminalServerMessage;
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, SqliteProjectWorkContextRepository,
        SqliteWorktreeRepository,
    };
    use ora_domain::{ProjectWorkContextSurface, WorktreeId};
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::path::Path;
    use tempfile::TempDir;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;
    use tower::util::ServiceExt;

    /// Verifies the liveness route reports process health without bootstrap state.
    #[tokio::test]
    async fn serves_liveness_route() {
        let (_temp_dir, _database_path, app) = test_router();
        let response = match app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health/live")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Verifies readiness stays unavailable until bootstrap marks the state as ready.
    #[tokio::test]
    async fn serves_unready_status_before_bootstrap_completion() {
        let temp_dir = TempDir::new().unwrap();
        let database_path = temp_dir.path().join("ready.sqlite3");
        let project_root = initialize_git_repository(temp_dir.path().join("repo"));
        let work_dir = temp_dir.path().join("worktrees");
        let app_state = build_app_state_for_database(&database_path, &project_root, &work_dir)
            .unwrap_or_else(|error| {
                panic!("expected application state bootstrap to succeed: {error}");
            });
        let app = build_router(app_state);
        let response = match app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health/ready")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    /// Verifies the router supports the persisted project CRUD slice end to end.
    #[tokio::test]
    async fn serves_project_crud_routes() {
        let (_temp_dir, _database_path, app) = test_router();
        let create_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "Ora",
                            "rootPath": "/workspace/ora",
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let created_project = response_json(create_response).await["project"].clone();
        let project_id = match created_project["id"].as_str() {
            Some(project_id) => project_id.to_string(),
            None => panic!("response did not include a project id"),
        };
        let list_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/projects")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let get_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/projects/{project_id}"))
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let update_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!("/api/projects/{project_id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "Ora Updated",
                            "rootPath": "/workspace/ora-next",
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let delete_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/projects/{project_id}"))
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(
            created_project,
            json!({
                "id": project_id,
                "name": "Ora",
                "rootPath": "/workspace/ora",
            })
        );
        assert_eq!(
            response_json(list_response).await,
            json!({
                "projects": [
                    {
                        "id": project_id,
                        "name": "Ora",
                        "rootPath": "/workspace/ora",
                    },
                ],
            })
        );
        assert_eq!(
            response_json(get_response).await,
            json!({
                "project": {
                    "id": project_id,
                    "name": "Ora",
                    "rootPath": "/workspace/ora",
                },
            })
        );
        assert_eq!(
            response_json(update_response).await,
            json!({
                "project": {
                    "id": project_id,
                    "name": "Ora Updated",
                    "rootPath": "/workspace/ora-next",
                },
            })
        );
        assert_eq!(
            response_json(delete_response).await,
            json!({
                "projectId": project_id,
            })
        );
    }

    /// Verifies missing projects surface the shared HTTP error payload.
    #[tokio::test]
    async fn serves_not_found_payload_for_missing_project() {
        let (_temp_dir, _database_path, app) = test_router();
        let response = match app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/projects/missing-project")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response_json(response).await,
            json!({
                "error": {
                    "code": "project_not_found",
                    "message": "project not found: missing-project",
                },
            })
        );
    }

    /// Verifies the router supports open, switch, and renew flows for project work contexts.
    #[tokio::test]
    async fn serves_project_work_context_routes() {
        let (_temp_dir, database_path, app) = test_router();
        let first_project_id = create_project(&app, "Ora", "/workspace/ora").await;
        let second_project_id = create_project(&app, "Ora Docs", "/workspace/ora-docs").await;
        let open_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/project-work-contexts/open")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "surface": "tauri",
                            "windowId": "window-1",
                            "projectId": first_project_id,
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let opened_context = response_json(open_response).await["context"].clone();
        let renew_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/project-work-contexts/renew")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "surface": "tauri",
                            "windowId": "window-1",
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let renewed_context = response_json(renew_response).await["context"].clone();
        let switch_response = match app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/project-work-contexts/open")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "surface": "tauri",
                            "windowId": "window-1",
                            "projectId": second_project_id,
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let switched_context = response_json(switch_response).await["context"].clone();

        assert_eq!(opened_context["windowId"], json!("window-1"));
        assert_eq!(opened_context["surface"], json!("tauri"));
        assert_eq!(opened_context["projectId"], json!(first_project_id));
        assert_eq!(renewed_context["id"], opened_context["id"]);
        assert_eq!(renewed_context["projectId"], json!(first_project_id));
        assert_eq!(switched_context["id"], opened_context["id"]);
        assert_eq!(switched_context["projectId"], json!(second_project_id));

        let repository = bootstrapped_project_work_context_repository(&database_path);
        assert_eq!(
            repository
                .find_project_work_context(ProjectWorkContextSurface::Tauri, "window-1")
                .unwrap()
                .map(|context| context.project_id.to_string()),
            Some(second_project_id)
        );
    }

    /// Verifies occupied projects surface the stable conflict payload for different Tauri windows.
    #[tokio::test]
    async fn serves_project_work_context_conflicts() {
        let (_temp_dir, _database_path, app) = test_router();
        let project_id = create_project(&app, "Ora", "/workspace/ora").await;

        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/project-work-contexts/open")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "surface": "tauri",
                            "windowId": "window-a",
                            "projectId": project_id,
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
            .unwrap_or_else(|error| panic!("request failed: {error}"));

        let conflict_response = match app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/project-work-contexts/open")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "surface": "tauri",
                            "windowId": "window-b",
                            "projectId": project_id,
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(conflict_response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response_json(conflict_response).await,
            json!({
                "error": {
                    "code": "project_occupied",
                    "message": format!("project is already occupied: {project_id}"),
                },
            })
        );
    }

    /// Verifies expired contexts stop blocking project opens once their lease is stale.
    #[tokio::test]
    async fn serves_project_work_context_recovery_after_expiry() {
        let (_temp_dir, database_path, app) = test_router();
        let project_id = create_project(&app, "Ora", "/workspace/ora").await;

        let open_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/project-work-contexts/open")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "surface": "tauri",
                            "windowId": "window-a",
                            "projectId": project_id,
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let opened_context = response_json(open_response).await["context"].clone();
        let repository = bootstrapped_project_work_context_repository(&database_path);
        let expired_context = repository
            .find_project_work_context(ProjectWorkContextSurface::Tauri, "window-a")
            .unwrap()
            .unwrap_or_else(|| panic!("expected context to exist after open"));

        repository
            .update_project_work_context(ora_domain::ProjectWorkContext::new(
                expired_context.id,
                expired_context.surface,
                expired_context.window_id,
                expired_context.project_id,
                0,
                expired_context.created_at,
                expired_context.updated_at,
            ))
            .unwrap();

        let recovery_response = match app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/project-work-contexts/open")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "surface": "tauri",
                            "windowId": "window-b",
                            "projectId": project_id,
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(recovery_response.status(), StatusCode::OK);
        assert_eq!(
            response_json(recovery_response).await["context"]["windowId"],
            json!("window-b")
        );
        assert_eq!(opened_context["windowId"], json!("window-a"));
    }

    /// Verifies the router supports task CRUD routes end to end.
    #[tokio::test]
    async fn serves_task_crud_routes() {
        let (_temp_dir, _database_path, app) = test_router();
        let create_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/tasks")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "projectId": "project-1",
                            "title": "Ship handlers",
                            "status": "todo",
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let created_task = response_json(create_response).await["task"].clone();
        let task_id = match created_task["id"].as_str() {
            Some(task_id) => task_id.to_string(),
            None => panic!("response did not include a task id"),
        };
        let list_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/tasks")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let get_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/tasks/{task_id}"))
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let update_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!("/api/tasks/{task_id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "projectId": "project-2",
                            "title": "Ship updated handlers",
                            "status": "doing",
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let repository = bootstrapped_worktree_repository(&_database_path);
        let worktree_id = match repository.list_worktrees().unwrap().first() {
            Some(worktree) => worktree.id.to_string(),
            None => panic!("expected created task worktree to exist before task deletion"),
        };
        let delete_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/tasks/{task_id}"))
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(
            created_task,
            json!({
                "id": task_id,
                "projectId": "project-1",
                "title": "Ship handlers",
                "status": "todo",
            })
        );
        assert_eq!(
            response_json(list_response).await,
            json!({
                "tasks": [
                    {
                        "id": task_id,
                        "projectId": "project-1",
                        "title": "Ship handlers",
                        "status": "todo",
                    },
                ],
            })
        );
        assert_eq!(
            response_json(get_response).await,
            json!({
                "task": {
                    "id": task_id,
                    "projectId": "project-1",
                    "title": "Ship handlers",
                    "status": "todo",
                },
            })
        );
        assert_eq!(
            response_json(update_response).await,
            json!({
                "task": {
                    "id": task_id,
                    "projectId": "project-2",
                    "title": "Ship updated handlers",
                    "status": "doing",
                },
            })
        );
        assert_eq!(
            response_json(delete_response).await,
            json!({
                "taskId": task_id,
            })
        );
        assert_eq!(
            repository
                .find_worktree(&WorktreeId::new(worktree_id))
                .unwrap(),
            None
        );
    }

    /// Verifies the router no longer exposes standalone public worktree routes.
    #[tokio::test]
    async fn rejects_public_worktree_routes() {
        let (_temp_dir, _database_path, app) = test_router();
        let collection_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/worktrees")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let item_response = match app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/worktrees/worktree-1")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(collection_response.status(), StatusCode::NOT_FOUND);
        assert_eq!(item_response.status(), StatusCode::NOT_FOUND);
    }

    /// Verifies the router supports session CRUD routes end to end.
    #[tokio::test]
    async fn serves_session_crud_routes() {
        let (_temp_dir, _database_path, app) = test_router();
        let create_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/sessions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "taskId": "task-1",
                            "agentId": "agent-1",
                            "agentSessionId": "provider-1",
                            "status": "running",
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let created_session = response_json(create_response).await["session"].clone();
        let session_id = match created_session["id"].as_str() {
            Some(session_id) => session_id.to_string(),
            None => panic!("response did not include a session id"),
        };
        let list_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/sessions")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let get_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/sessions/{session_id}"))
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let update_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!("/api/sessions/{session_id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "taskId": "task-2",
                            "agentId": "agent-2",
                            "agentSessionId": null,
                            "status": "stopped",
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let delete_response = match app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/sessions/{session_id}"))
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(
            created_session,
            json!({
                "id": session_id,
                "taskId": "task-1",
                "agentId": "agent-1",
                "agentSessionId": "provider-1",
                "status": "running",
            })
        );
        assert_eq!(
            response_json(list_response).await,
            json!({
                "sessions": [
                    {
                        "id": session_id,
                        "taskId": "task-1",
                        "agentId": "agent-1",
                        "agentSessionId": "provider-1",
                        "status": "running",
                    },
                ],
            })
        );
        assert_eq!(
            response_json(get_response).await,
            json!({
                "session": {
                    "id": session_id,
                    "taskId": "task-1",
                    "agentId": "agent-1",
                    "agentSessionId": "provider-1",
                    "status": "running",
                },
            })
        );
        assert_eq!(
            response_json(update_response).await,
            json!({
                "session": {
                    "id": session_id,
                    "taskId": "task-2",
                    "agentId": "agent-2",
                    "agentSessionId": null,
                    "status": "stopped",
                },
            })
        );
        assert_eq!(
            response_json(delete_response).await,
            json!({
                "sessionId": session_id,
            })
        );
    }

    /// Verifies terminal sessions stay running across WebSocket disconnects and replay buffered output on reconnect.
    #[tokio::test]
    async fn serves_terminal_websocket_lifecycle() {
        let (temp_dir, _database_path, app_state, base_http_url, base_ws_url, server_task) =
            start_test_server().await;
        let project_id = create_project_via_http(&base_http_url, "Ora", "/workspace/ora").await;
        let task_id = create_task_via_http(&base_http_url, &project_id, "Ship task terminal").await;
        let create_response = http_json(
            reqwest::Client::new(),
            reqwest::Method::POST,
            format!("{base_http_url}/api/sessions"),
            Some(json!({
                "taskId": task_id,
                "agentId": "terminal",
                "agentSessionId": null,
                "status": "running",
                "terminal": {
                    "cols": 100,
                    "rows": 30,
                },
            })),
        )
        .await;
        let session_id = create_response["session"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("expected created session id"))
            .to_string();
        let (mut first_socket, _) = tokio_tungstenite::connect_async(format!(
            "{base_ws_url}/api/sessions/{session_id}/terminal"
        ))
        .await
        .unwrap_or_else(|error| panic!("expected terminal websocket connect to succeed: {error}"));

        assert_eq!(
            read_terminal_message(&mut first_socket).await,
            TerminalServerMessage::Ready {
                session_id: session_id.clone(),
            }
        );
        drop(first_socket);

        assert_eq!(
            http_json(
                reqwest::Client::new(),
                reqwest::Method::GET,
                format!("{base_http_url}/api/sessions/{session_id}"),
                None,
            )
            .await["session"]["status"],
            json!("running")
        );
        app_state.shutdown_terminals();
        server_task.abort();
        drop(temp_dir);
    }

    /// Verifies the terminal route rejects duplicate live attachments for the same running session.
    #[tokio::test]
    async fn rejects_duplicate_terminal_attachment() {
        let (temp_dir, _database_path, app_state, base_http_url, base_ws_url, server_task) =
            start_test_server().await;
        let project_id = create_project_via_http(&base_http_url, "Ora", "/workspace/ora").await;
        let task_id =
            create_task_via_http(&base_http_url, &project_id, "Ship duplicate attach").await;
        let session_id = create_terminal_session_via_http(&base_http_url, &task_id).await;
        let (_first_socket, _) = tokio_tungstenite::connect_async(format!(
            "{base_ws_url}/api/sessions/{session_id}/terminal"
        ))
        .await
        .unwrap_or_else(|error| {
            panic!("expected first terminal websocket connect to succeed: {error}")
        });
        let error = tokio_tungstenite::connect_async(format!(
            "{base_ws_url}/api/sessions/{session_id}/terminal"
        ))
        .await
        .unwrap_err();

        assert!(
            error.to_string().contains("409"),
            "expected duplicate attach to fail with HTTP 409, got {error}"
        );
        app_state.shutdown_terminals();
        server_task.abort();
        drop(temp_dir);
    }

    /// Verifies the terminal route rejects non-terminal sessions before upgrading the socket.
    #[tokio::test]
    async fn rejects_non_terminal_session_attachment() {
        let (temp_dir, _database_path, app_state, base_http_url, base_ws_url, server_task) =
            start_test_server().await;
        let create_response = http_json(
            reqwest::Client::new(),
            reqwest::Method::POST,
            format!("{base_http_url}/api/sessions"),
            Some(json!({
                "taskId": "task-1",
                "agentId": "agent-1",
                "agentSessionId": null,
                "status": "running",
            })),
        )
        .await;
        let session_id = create_response["session"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("expected created session id"));
        let error = tokio_tungstenite::connect_async(format!(
            "{base_ws_url}/api/sessions/{session_id}/terminal"
        ))
        .await
        .unwrap_err();

        assert!(
            error.to_string().contains("409"),
            "expected non-terminal attach to fail with HTTP 409, got {error}"
        );
        app_state.shutdown_terminals();
        server_task.abort();
        drop(temp_dir);
    }

    /// Builds a ready router for tests that need the full persisted route surface.
    fn test_router() -> (TempDir, std::path::PathBuf, axum::Router) {
        let temp_dir = TempDir::new().unwrap();
        let database_path = temp_dir.path().join("routes.sqlite3");
        let project_root = initialize_git_repository(temp_dir.path().join("repo"));
        let work_dir = temp_dir.path().join("worktrees");
        let app_state = build_app_state_for_database(&database_path, &project_root, &work_dir)
            .unwrap_or_else(|error| {
                panic!("expected application state bootstrap to succeed: {error}");
            });
        app_state.mark_ready();

        (temp_dir, database_path, build_router(app_state))
    }

    /// Starts one real HTTP server so terminal WebSocket tests can exercise upgrade flows end to end.
    async fn start_test_server() -> (
        TempDir,
        std::path::PathBuf,
        AppState,
        String,
        String,
        tokio::task::JoinHandle<()>,
    ) {
        let temp_dir = TempDir::new().unwrap();
        let database_path = temp_dir.path().join("routes-terminal.sqlite3");
        let project_root = initialize_git_repository(temp_dir.path().join("repo"));
        let work_dir = temp_dir.path().join("worktrees");
        let app_state = build_app_state_for_database(&database_path, &project_root, &work_dir)
            .unwrap_or_else(|error| {
                panic!("expected application state bootstrap to succeed: {error}");
            });
        app_state.mark_ready();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|error| panic!("expected test listener bind to succeed: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("expected listener address lookup to succeed: {error}"));
        let app = build_router(app_state.clone());
        let server_task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .unwrap_or_else(|error| panic!("expected test server to keep serving: {error}"));
        });

        (
            temp_dir,
            database_path,
            app_state,
            format!("http://{address}"),
            format!("ws://{address}"),
            server_task,
        )
    }

    /// Creates one project through the HTTP API and returns the generated project id.
    async fn create_project(app: &axum::Router, name: &str, root_path: &str) -> String {
        let create_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": name,
                            "rootPath": root_path,
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        match response_json(create_response).await["project"]["id"].as_str() {
            Some(project_id) => project_id.to_string(),
            None => panic!("response did not include a project id"),
        }
    }

    /// Creates one project through the live HTTP server and returns the generated project id.
    async fn create_project_via_http(base_http_url: &str, name: &str, root_path: &str) -> String {
        http_json(
            reqwest::Client::new(),
            reqwest::Method::POST,
            format!("{base_http_url}/api/projects"),
            Some(json!({
                "name": name,
                "rootPath": root_path,
            })),
        )
        .await["project"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("expected created project id"))
            .to_string()
    }

    /// Creates one task through the live HTTP server and returns the generated task id.
    async fn create_task_via_http(base_http_url: &str, project_id: &str, title: &str) -> String {
        http_json(
            reqwest::Client::new(),
            reqwest::Method::POST,
            format!("{base_http_url}/api/tasks"),
            Some(json!({
                "projectId": project_id,
                "title": title,
                "status": "doing",
            })),
        )
        .await["task"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("expected created task id"))
            .to_string()
    }

    /// Creates one terminal session through the live HTTP server and returns the generated session id.
    async fn create_terminal_session_via_http(base_http_url: &str, task_id: &str) -> String {
        http_json(
            reqwest::Client::new(),
            reqwest::Method::POST,
            format!("{base_http_url}/api/sessions"),
            Some(json!({
                "taskId": task_id,
                "agentId": "terminal",
                "agentSessionId": null,
                "status": "running",
                "terminal": {
                    "cols": 80,
                    "rows": 24,
                },
            })),
        )
        .await["session"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("expected created session id"))
            .to_string()
    }

    /// Opens the test database so route assertions can inspect persisted work context state.
    fn bootstrapped_project_work_context_repository(
        database_path: &Path,
    ) -> SqliteProjectWorkContextRepository {
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(database_path),
                &ora_db::default_migration_catalog().unwrap(),
            )
            .unwrap_or_else(|error| {
                panic!("expected repository pool bootstrap to succeed: {error}")
            });

        SqliteProjectWorkContextRepository::new(pool)
    }

    /// Opens the test database so route assertions can inspect persisted worktree state.
    fn bootstrapped_worktree_repository(database_path: &Path) -> SqliteWorktreeRepository {
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(database_path),
                &ora_db::default_migration_catalog().unwrap(),
            )
            .unwrap_or_else(|error| {
                panic!("expected repository pool bootstrap to succeed: {error}")
            });

        SqliteWorktreeRepository::new(pool)
    }

    /// Initializes one real Git repository with an initial commit so task routes can exercise linked worktree creation.
    fn initialize_git_repository(repository_root: std::path::PathBuf) -> std::path::PathBuf {
        std::fs::create_dir_all(&repository_root)
            .unwrap_or_else(|error| panic!("failed to create repository root: {error}"));
        run_git(&repository_root, &["init", "--initial-branch=main"]);
        run_git(&repository_root, &["config", "user.name", "Ora Tests"]);
        run_git(
            &repository_root,
            &["config", "user.email", "ora-tests@example.com"],
        );
        std::fs::write(repository_root.join("README.md"), "ora test repo\n")
            .unwrap_or_else(|error| panic!("failed to write repository file: {error}"));
        run_git(&repository_root, &["add", "README.md"]);
        run_git(&repository_root, &["commit", "-m", "initial"]);

        repository_root
    }

    /// Runs one Git command for route-test repository setup and fails loudly when bootstrap assumptions are broken.
    fn run_git(repository_root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .current_dir(repository_root)
            .args(args)
            .status()
            .unwrap_or_else(|error| panic!("failed to start git {:?}: {error}", args));

        assert!(
            status.success(),
            "git {:?} failed with status {status}",
            args
        );
    }

    /// Decodes one JSON response body so route tests can compare the full payload.
    async fn response_json(response: axum::response::Response) -> Value {
        let bytes = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(bytes) => bytes,
            Err(error) => panic!("failed to read response body: {error}"),
        };

        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(error) => panic!("failed to decode JSON body: {error}"),
        }
    }

    /// Sends one JSON HTTP request to the live test server and returns the decoded JSON payload.
    async fn http_json(
        client: reqwest::Client,
        method: reqwest::Method,
        url: String,
        body: Option<Value>,
    ) -> Value {
        let request = client.request(method, url);
        let request = match body {
            Some(body) => request.json(&body),
            None => request,
        };
        let response = request
            .send()
            .await
            .unwrap_or_else(|error| panic!("expected live HTTP request to succeed: {error}"));
        let status = response.status();
        let actual = response
            .json::<Value>()
            .await
            .unwrap_or_else(|error| panic!("expected live HTTP JSON body to decode: {error}"));

        assert!(
            status.is_success(),
            "expected HTTP success, got {status} with {actual}"
        );

        actual
    }

    /// Reads one terminal server message from the socket and decodes the shared protocol payload.
    async fn read_terminal_message(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> TerminalServerMessage {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => serde_json::from_str(&text)
                .unwrap_or_else(|error| panic!("expected terminal server JSON payload: {error}")),
            Some(Ok(other)) => panic!("expected terminal text message, got {other:?}"),
            Some(Err(error)) => panic!("expected terminal message read to succeed: {error}"),
            None => panic!("expected terminal message but socket closed"),
        }
    }

}
