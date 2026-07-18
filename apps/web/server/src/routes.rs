use crate::app_state::AppState;
use crate::handlers::{
    agents, file_system, health, project_work_contexts, projects, sessions, skills, tasks,
};
use axum::Router;
use axum::routing::{get, post};
use ora_contracts::{
    AGENT_PATH, AGENTS_PATH, FILE_SYSTEM_DIRECTORY_PATH, PROJECT_PATH,
    PROJECT_WORK_CONTEXT_OPEN_PATH, PROJECT_WORK_CONTEXT_RENEW_PATH, PROJECTS_PATH, SESSION_PATH,
    SESSIONS_PATH, SKILL_PATH, SKILLS_PATH, TASK_PATH, TASKS_PATH,
};

/// Builds the top-level router for health checks and the persisted CRUD routes.
pub fn build_router(app_state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(health::liveness))
        .route("/health/ready", get(health::readiness))
        .route(FILE_SYSTEM_DIRECTORY_PATH, get(file_system::list_directory))
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
            SKILLS_PATH,
            post(skills::create_skill).get(skills::list_skills),
        )
        .route(
            SKILL_PATH,
            get(skills::get_skill)
                .put(skills::update_skill)
                .delete(skills::delete_skill),
        )
        .route(
            AGENTS_PATH,
            post(agents::create_agent).get(agents::list_agents),
        )
        .route(
            AGENT_PATH,
            get(agents::get_agent)
                .put(agents::update_agent)
                .delete(agents::delete_agent),
        )
        .with_state(app_state)
}

#[cfg(test)]
mod tests {
    use super::build_router;
    use crate::bootstrap::build_app_state_for_database;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use ora_application::{ProjectWorkContextRepository, WorktreeRepository};
    use ora_contracts::{
        FileSystemBreadcrumb, FileSystemEntry, FileSystemEntryKind, ListDirectoryResponse,
    };
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, SqliteProjectWorkContextRepository,
        SqliteWorktreeRepository,
    };
    use ora_domain::{ProjectWorkContextSurface, WorktreeId};
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;
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

    /// Verifies the filesystem route lists an explicit absolute directory through its GET query.
    #[tokio::test]
    async fn serves_file_system_directory_listings() {
        let (temp_dir, _database_path, app) = test_router();
        let directory = temp_dir.path().join("browser-fixture");
        fs::create_dir(&directory).unwrap_or_else(|error| {
            panic!("failed to create browser fixture directory: {error}");
        });
        fs::create_dir(directory.join("project")).unwrap_or_else(|error| {
            panic!("failed to create project fixture directory: {error}");
        });
        fs::write(directory.join("README.md"), "fixture").unwrap_or_else(|error| {
            panic!("failed to create browser fixture file: {error}");
        });
        let uri = format!(
            "/api/file-system/directory?path={}",
            directory.to_string_lossy()
        );

        let response = request_empty(&app, Method::GET, &uri).await;
        let status = response.status();
        let actual = serde_json::from_value::<ListDirectoryResponse>(response_json(response).await)
            .unwrap_or_else(|error| panic!("failed to decode directory response: {error}"));
        let mut breadcrumbs = directory
            .ancestors()
            .map(|path| FileSystemBreadcrumb {
                name: path.file_name().map_or_else(
                    || path.to_string_lossy().to_string(),
                    |name| name.to_string_lossy().to_string(),
                ),
                path: path.to_string_lossy().to_string(),
            })
            .collect::<Vec<_>>();
        breadcrumbs.reverse();

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            actual,
            ListDirectoryResponse {
                current_path: directory.to_string_lossy().to_string(),
                parent_path: directory
                    .parent()
                    .map(|path| path.to_string_lossy().to_string()),
                breadcrumbs,
                entries: vec![
                    FileSystemEntry {
                        name: "project".to_string(),
                        path: directory.join("project").to_string_lossy().to_string(),
                        kind: FileSystemEntryKind::Directory,
                        is_symbolic_link: false,
                    },
                    FileSystemEntry {
                        name: "README.md".to_string(),
                        path: directory.join("README.md").to_string_lossy().to_string(),
                        kind: FileSystemEntryKind::File,
                        is_symbolic_link: false,
                    },
                ],
            }
        );
    }

    /// Verifies relative filesystem queries receive the stable invalid-path response.
    #[tokio::test]
    async fn rejects_relative_file_system_paths() {
        let (_temp_dir, _database_path, app) = test_router();
        let response = request_empty(
            &app,
            Method::GET,
            "/api/file-system/directory?path=relative",
        )
        .await;
        let status = response.status();
        let body = response_json(response).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({
                "error": {
                    "code": "invalid_file_system_path",
                    "message": "filesystem path must be absolute: \"relative\"",
                },
            })
        );
    }

    /// Verifies the router supports the persisted project CRUD slice end to end.
    #[tokio::test]
    async fn serves_project_crud_routes() {
        let (temp_dir, _database_path, app) = test_router();
        let project_root = workspace_project_root(&temp_dir, "ora");
        let updated_project_root = workspace_project_root(&temp_dir, "ora-next");
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
                            "rootPath": project_root.clone(),
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
                            "rootPath": updated_project_root.clone(),
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
                "rootPath": project_root.clone(),
            })
        );
        assert_eq!(
            response_json(list_response).await,
            json!({
                "projects": [
                    {
                        "id": project_id,
                        "name": "Ora",
                            "rootPath": project_root.clone(),
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
                    "rootPath": project_root.clone(),
                },
            })
        );
        assert_eq!(
            response_json(update_response).await,
            json!({
                "project": {
                    "id": project_id,
                    "name": "Ora Updated",
                        "rootPath": updated_project_root.clone(),
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
        let (temp_dir, database_path, app) = test_router();
        let first_project_root = workspace_project_root(&temp_dir, "ora");
        let second_project_root = workspace_project_root(&temp_dir, "ora-docs");
        let first_project_id = create_project(&app, "Ora", &first_project_root).await;
        let second_project_id = create_project(&app, "Ora Docs", &second_project_root).await;
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
        let (temp_dir, _database_path, app) = test_router();
        let project_root = workspace_project_root(&temp_dir, "ora");
        let project_id = create_project(&app, "Ora", &project_root).await;

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
        let (temp_dir, database_path, app) = test_router();
        let project_root = workspace_project_root(&temp_dir, "ora");
        let project_id = create_project(&app, "Ora", &project_root).await;

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

    /// Verifies catalog routes address resources by identifier while names remain editable fields.
    #[tokio::test]
    async fn serves_skill_and_agent_crud_routes() {
        let (_temp_dir, _database_path, app) = test_router();
        let skill_create = request_json(
            &app,
            Method::POST,
            "/api/skills",
            json!({ "name": " review / guide ", "description": "Reviews guides" }),
        )
        .await;
        assert_eq!(skill_create.status(), StatusCode::OK);
        let skill = response_json(skill_create).await;
        let skill_id = skill["skill"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("response did not include a skill id"))
            .to_string();
        assert_eq!(skill["skill"]["name"], "review / guide");
        let skill_path = format!("/api/skills/{skill_id}");
        let skill_get = request_empty(&app, Method::GET, &skill_path).await;
        assert_eq!(skill_get.status(), StatusCode::OK);
        let skill_list = request_empty(&app, Method::GET, "/api/skills").await;
        assert_eq!(skill_list.status(), StatusCode::OK);
        let skill_update = request_json(
            &app,
            Method::PUT,
            &skill_path,
            json!({ "name": "reviewer", "description": "Reviews changes" }),
        )
        .await;
        assert_eq!(skill_update.status(), StatusCode::OK);
        assert_eq!(
            response_json(skill_update).await,
            json!({ "skill": { "id": skill_id, "name": "reviewer", "description": "Reviews changes" } })
        );
        let duplicate_skill = request_json(
            &app,
            Method::POST,
            "/api/skills",
            json!({ "name": "reviewer", "description": "Duplicate" }),
        )
        .await;
        assert_eq!(duplicate_skill.status(), StatusCode::OK);
        let skill_delete = request_empty(&app, Method::DELETE, &skill_path).await;
        assert_eq!(skill_delete.status(), StatusCode::OK);
        assert_eq!(
            request_empty(&app, Method::GET, &skill_path).await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            request_json(
                &app,
                Method::POST,
                "/api/skills",
                json!({ "name": "   ", "description": "Invalid" })
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );

        let agent_create = request_json(
            &app,
            Method::POST,
            "/api/agents",
            json!({ "name": "opencode", "description": "OpenCode" }),
        )
        .await;
        assert_eq!(agent_create.status(), StatusCode::OK);
        let agent = response_json(agent_create).await;
        let agent_id = agent["agent"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("response did not include an agent id"))
            .to_string();
        let agent_path = format!("/api/agents/{agent_id}");
        assert_eq!(
            request_empty(&app, Method::GET, "/api/agents")
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            request_empty(&app, Method::GET, &agent_path).await.status(),
            StatusCode::OK
        );
        let agent_update = request_json(
            &app,
            Method::PUT,
            &agent_path,
            json!({ "name": "review-agent", "description": "Reviews changes" }),
        )
        .await;
        assert_eq!(agent_update.status(), StatusCode::OK);
        assert_eq!(
            request_empty(&app, Method::DELETE, &agent_path)
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            request_empty(&app, Method::GET, &agent_path).await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            request_json(
                &app,
                Method::POST,
                "/api/agents",
                json!({ "name": " ", "description": "Invalid" })
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }

    /// Sends one JSON request to the router under test.
    async fn request_json(
        app: &axum::Router,
        method: Method,
        uri: &str,
        body: Value,
    ) -> axum::response::Response {
        match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        }
    }

    /// Sends one empty-body request to the router under test.
    async fn request_empty(
        app: &axum::Router,
        method: Method,
        uri: &str,
    ) -> axum::response::Response {
        match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        }
    }

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

    /// Derives one temp-directory-backed project root for route test fixtures.
    fn workspace_project_root(temp_dir: &TempDir, name: &str) -> String {
        temp_dir
            .path()
            .join("workspace")
            .join(name)
            .to_string_lossy()
            .to_string()
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
}
