use crate::app_state::AppState;
use crate::handlers::{
    agents, file_system, git, health, plugins, project_work_contexts, projects, sessions,
    skill_imports, skills, snapshots, specs, task_diffs, tasks, workflow_runs, workflows,
    workspace_files,
};
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{get, post};
use ora_contracts::{
    AGENT_IMPORT_COMMIT_PATH, AGENT_IMPORT_PREPARE_PATH, AGENT_PATH, AGENT_RUNTIME_STATUS_PATH,
    AGENTS_PATH, APP_EVENT_WATCH_PATH, FILE_SYSTEM_DIRECTORY_PATH, GIT_IDENTITY_PATH,
    INSTALLED_PLUGINS_PATH, PROJECT_BRANCHES_PATH, PROJECT_PATH, PROJECT_SPEC_SOURCES_PATH,
    PROJECT_WORK_CONTEXT_OPEN_PATH, PROJECT_WORK_CONTEXT_RENEW_PATH, PROJECTS_PATH,
    SESSION_ATTACH_PATH, SESSION_CONFIG_PATH, SESSION_LOAD_PATH, SESSION_PATH,
    SESSION_PERMISSION_RESPONSE_PATH, SESSION_PROMPT_PATH, SESSION_RESUME_HISTORY_PATH,
    SESSION_STOP_PATH, SESSION_SWITCH_AGENT_PATH, SESSION_WARM_PATH, SESSIONS_PATH,
    SKILL_IMPORT_COMMIT_PATH, SKILL_IMPORT_PATH, SKILL_IMPORTS_PATH, SKILL_PATH, SKILLS_PATH,
    SPEC_CATALOG_PATH, SPEC_READ_PATH, SPEC_RESOLVE_SOURCE_PATH, SPEC_WATCH_PATH, TASK_COMMIT_PATH,
    TASK_DIFF_COMMENT_REPLIES_PATH, TASK_DIFF_COMMENT_STATUS_PATH, TASK_DIFF_COMMENTS_PATH,
    TASK_DIFF_PATH, TASK_PATH, TASK_PUSH_PATH, TASK_WORKSPACE_PATH, TASKS_PATH,
    WORKFLOW_ACTIVATE_PATH, WORKFLOW_DRAFT_PATH, WORKFLOW_PATH, WORKFLOW_PUBLISH_PATH,
    WORKFLOW_ROLLBACK_PATH, WORKFLOW_RUN_CANCEL_PATH, WORKFLOW_RUN_INPUT_PATH,
    WORKFLOW_RUN_NODES_PATH, WORKFLOW_RUN_PATH, WORKFLOW_RUN_RESTART_PATH, WORKFLOW_RUN_START_PATH,
    WORKFLOW_RUNS_PATH, WORKFLOW_SNAPSHOT_PATH, WORKFLOW_VERSION_PATH, WORKFLOW_VERSIONS_PATH,
    WORKFLOWS_PATH, WORKSPACE_DIRECTORY_PATH, WORKSPACE_FILE_PATH, WORKSPACE_SEARCH_PATH,
    WORKSPACE_WATCH_PATH,
};
use tower_http::cors::CorsLayer;
use tower_http::request_id::PropagateRequestIdLayer;

/// Builds the top-level router for health checks and the persisted CRUD routes.
pub fn build_router(app_state: AppState) -> Router {
    Router::new()
        // =============================================================================
        // health
        // =============================================================================
        .route("/health/live", get(health::liveness))
        .route("/health/ready", get(health::readiness))
        // =============================================================================
        // project
        // =============================================================================
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
        .route(PROJECT_BRANCHES_PATH, get(projects::list_project_branches))
        // =============================================================================
        // projectWorkContext
        // =============================================================================
        .route(
            PROJECT_WORK_CONTEXT_OPEN_PATH,
            post(project_work_contexts::open_project_work_context),
        )
        .route(
            PROJECT_WORK_CONTEXT_RENEW_PATH,
            post(project_work_contexts::renew_project_work_context),
        )
        // =============================================================================
        // task
        // =============================================================================
        .route(TASKS_PATH, post(tasks::create_task).get(tasks::list_tasks))
        .route(
            TASK_PATH,
            get(tasks::get_task)
                .put(tasks::update_task)
                .delete(tasks::delete_task),
        )
        .route(TASK_WORKSPACE_PATH, get(tasks::get_task_workspace))
        // =============================================================================
        // spec
        // =============================================================================
        .route(SPEC_CATALOG_PATH, post(specs::catalog))
        .route(SPEC_READ_PATH, post(specs::read))
        .route(SPEC_RESOLVE_SOURCE_PATH, post(specs::resolve_source))
        .route(
            PROJECT_SPEC_SOURCES_PATH,
            axum::routing::put(specs::update_project_sources),
        )
        .route(SPEC_WATCH_PATH, post(specs::watch))
        // =============================================================================
        // taskDiff
        // =============================================================================
        .route(TASK_DIFF_PATH, get(task_diffs::get_task_diff))
        .route(TASK_COMMIT_PATH, post(task_diffs::commit_task_changes))
        .route(TASK_PUSH_PATH, post(task_diffs::push_task_branch))
        .route(
            TASK_DIFF_COMMENTS_PATH,
            post(task_diffs::create_task_diff_comment).get(task_diffs::list_task_diff_comments),
        )
        .route(
            TASK_DIFF_COMMENT_REPLIES_PATH,
            post(task_diffs::reply_task_diff_comment),
        )
        .route(
            TASK_DIFF_COMMENT_STATUS_PATH,
            axum::routing::put(task_diffs::set_task_diff_comment_status),
        )
        // =============================================================================
        // session
        // =============================================================================
        // The static warm path is registered before the identifier route so it is
        // never captured as a session id.
        .route(SESSION_WARM_PATH, post(sessions::warm_session))
        .route(SESSIONS_PATH, get(sessions::list_sessions))
        .route(APP_EVENT_WATCH_PATH, post(sessions::watch_app_events))
        .route(
            SESSION_PATH,
            get(sessions::get_session).delete(sessions::delete_session),
        )
        .route(SESSION_CONFIG_PATH, post(sessions::set_session_config))
        .route(SESSION_ATTACH_PATH, post(sessions::attach_session))
        .route(SESSION_LOAD_PATH, post(sessions::load_session))
        .route(SESSION_PROMPT_PATH, post(sessions::prompt_session))
        .route(
            SESSION_PERMISSION_RESPONSE_PATH,
            post(sessions::respond_to_permission),
        )
        .route(SESSION_STOP_PATH, post(sessions::stop_session))
        .route(
            SESSION_SWITCH_AGENT_PATH,
            post(sessions::switch_session_agent),
        )
        .route(
            SESSION_RESUME_HISTORY_PATH,
            post(sessions::resume_session_history),
        )
        // =============================================================================
        // agentRuntime
        // =============================================================================
        .route(
            AGENT_RUNTIME_STATUS_PATH,
            get(sessions::get_agent_runtime_status),
        )
        // =============================================================================
        // skill
        // =============================================================================
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
        .merge(skill_imports_router())
        // =============================================================================
        // agent
        // =============================================================================
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
        .route(
            AGENT_IMPORT_PREPARE_PATH,
            post(agents::prepare_agent_import),
        )
        .route(AGENT_IMPORT_COMMIT_PATH, post(agents::commit_agent_import))
        // =============================================================================
        // plugin
        // =============================================================================
        .route(INSTALLED_PLUGINS_PATH, get(plugins::list_installed_plugins))
        // =============================================================================
        // fileSystem
        // =============================================================================
        .route(FILE_SYSTEM_DIRECTORY_PATH, get(file_system::list_directory))
        .route(
            WORKSPACE_DIRECTORY_PATH,
            post(workspace_files::list_directory),
        )
        .route(WORKSPACE_FILE_PATH, post(workspace_files::read_file))
        .route(WORKSPACE_SEARCH_PATH, post(workspace_files::search))
        .route(WORKSPACE_WATCH_PATH, get(workspace_files::watch))
        // =============================================================================
        // gitIdentity
        // =============================================================================
        .route(GIT_IDENTITY_PATH, get(git::get_identity))
        // =============================================================================
        // workflow
        // =============================================================================
        .route(
            WORKFLOWS_PATH,
            post(workflows::create_workflow).get(workflows::list_workflows),
        )
        .route(
            WORKFLOW_PATH,
            get(workflows::get_workflow)
                .put(workflows::update_workflow)
                .delete(workflows::delete_workflow),
        )
        .route(
            WORKFLOW_DRAFT_PATH,
            get(workflows::get_draft).put(workflows::update_draft),
        )
        .route(WORKFLOW_PUBLISH_PATH, post(workflows::publish_workflow))
        .route(WORKFLOW_ROLLBACK_PATH, post(workflows::rollback_workflow))
        .route(WORKFLOW_ACTIVATE_PATH, post(workflows::activate_workflow))
        .route(WORKFLOW_VERSIONS_PATH, get(workflows::list_versions))
        .route(
            WORKFLOW_VERSION_PATH,
            get(workflows::get_version).delete(workflows::delete_snapshot),
        )
        .route(
            WORKFLOW_SNAPSHOT_PATH,
            get(snapshots::get_workflow_snapshot),
        )
        // =============================================================================
        // workflowRun
        // =============================================================================
        .route(
            WORKFLOW_RUNS_PATH,
            post(workflow_runs::create_workflow_run).get(workflow_runs::list_workflow_runs),
        )
        .route(
            WORKFLOW_RUN_PATH,
            get(workflow_runs::get_workflow_run).delete(workflow_runs::delete_workflow_run),
        )
        .route(
            WORKFLOW_RUN_NODES_PATH,
            get(workflow_runs::list_workflow_node_runs),
        )
        .route(
            WORKFLOW_RUN_START_PATH,
            post(workflow_runs::start_workflow_run),
        )
        .route(
            WORKFLOW_RUN_CANCEL_PATH,
            post(workflow_runs::cancel_workflow_run),
        )
        .route(
            WORKFLOW_RUN_RESTART_PATH,
            post(workflow_runs::restart_workflow_run),
        )
        .route(
            WORKFLOW_RUN_INPUT_PATH,
            post(workflow_runs::update_workflow_run_input),
        )
        .with_state(app_state)
        .layer(PropagateRequestIdLayer::new(crate::error::X_REQUEST_ID))
        .layer(middleware::from_fn(crate::error::request_context))
        .layer(CorsLayer::new().expose_headers([crate::error::X_REQUEST_ID]))
}

/// Builds the import endpoints with room for multipart framing over the 200 MiB file budget.
fn skill_imports_router() -> Router<AppState> {
    Router::new()
        .route(
            SKILL_IMPORTS_PATH,
            post(skill_imports::prepare_skill_import),
        )
        .route(
            SKILL_IMPORT_PATH,
            get(skill_imports::get_skill_import).delete(skill_imports::cancel_skill_import),
        )
        .route(
            SKILL_IMPORT_COMMIT_PATH,
            post(skill_imports::commit_skill_import),
        )
        .layer(DefaultBodyLimit::max(201 * 1024 * 1024))
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
        RequestId,
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

    /// Verifies the static warm route is matched ahead of the session id route.
    ///
    /// `/api/sessions/warm` and `/api/sessions/{sessionId}` overlap, so a
    /// regression in route order would silently turn every warm request into a
    /// lookup for a session named "warm". Reaching the handler is the assertion:
    /// warming itself needs a running agent CLI, which a test machine may lack.
    #[tokio::test]
    async fn routes_warm_requests_past_the_session_identifier_route() {
        let (_temp_dir, _database_path, app) = test_router();
        let response = request_json(
            &app,
            Method::POST,
            "/api/sessions/warm",
            json!({
                "target": { "type": "projectRoot", "projectId": "missing-project" },
                "agentCli": "open_code",
                "clientId": "client-1",
            }),
        )
        .await;

        assert_ne!(response.status(), StatusCode::NOT_FOUND);
        assert_ne!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    /// Verifies readiness stays unavailable until bootstrap marks the state as ready.
    #[tokio::test]
    async fn serves_unready_status_before_bootstrap_completion() {
        let temp_dir = TempDir::new().unwrap();
        let database_path = temp_dir.path().join("ready.sqlite3");
        let project_root = initialize_git_repository(temp_dir.path().join("repo"));
        let work_dir = temp_dir.path().join("worktrees");
        let app_state =
            build_app_state_for_database(&database_path, &project_root, &work_dir, temp_dir.path())
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
        assert_contract_error(&body, "file_system_path_not_absolute");
    }

    /// Verifies the router supports the persisted project CRUD slice end to end.
    #[tokio::test]
    async fn serves_project_crud_routes() {
        let (temp_dir, _database_path, app) = test_router();
        let project_root = workspace_project_root(&temp_dir, "ora");
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
                    "rootPath": project_root.clone(),
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
                    .header(
                        crate::error::X_REQUEST_ID,
                        "550e8400-e29b-41d4-a716-446655440000",
                    )
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let header_request_id = response
            .headers()
            .get(crate::error::X_REQUEST_ID)
            .and_then(|value| value.to_str().ok())
            .expect("response must expose X-Request-Id")
            .to_string();
        let body = response_json(response).await;
        assert_contract_error(&body, "project_not_found");
        assert_eq!(body["requestId"], header_request_id);
        assert_ne!(header_request_id, "550e8400-e29b-41d4-a716-446655440000");
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
        assert_contract_error(&response_json(conflict_response).await, "project_occupied");
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
        let project_id = create_project(
            &app,
            "Task project",
            &_temp_dir.path().join("repo").to_string_lossy(),
        )
        .await;
        let create_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/tasks")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "projectId": project_id,
                            "title": "Ship handlers",
                            "status": "todo",
                            "baseBranch": "main",
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
        let workspace_response = request_empty(
            &app,
            Method::GET,
            &format!("/api/tasks/{task_id}/workspace"),
        )
        .await;
        assert_eq!(workspace_response.status(), StatusCode::OK);
        let workspace = response_json(workspace_response).await["workspace"].clone();
        assert_eq!(workspace["branchName"], format!("ora/{}", &task_id[..8]));
        let worktree_root = std::path::PathBuf::from(
            workspace["rootPath"]
                .as_str()
                .expect("workspace response must include a root"),
        );
        let worktree_source = worktree_root.join("docs").join("specs");
        std::fs::create_dir_all(&worktree_source)
            .unwrap_or_else(|error| panic!("failed to create worktree Spec source: {error}"));
        std::fs::write(worktree_source.join("task.md"), "# Task Spec\n")
            .unwrap_or_else(|error| panic!("failed to write worktree Spec: {error}"));
        let task_catalog = request_json(
            &app,
            Method::POST,
            "/api/specs/catalog",
            json!({ "target": { "kind": "task", "taskId": task_id } }),
        )
        .await;
        assert_eq!(task_catalog.status(), StatusCode::OK);
        assert_eq!(
            response_json(task_catalog).await["documents"][0]["relativePath"],
            "docs/specs/task.md"
        );
        std::fs::remove_dir_all(worktree_root.join("docs"))
            .unwrap_or_else(|error| panic!("failed to clean worktree Spec fixture: {error}"));
        let branch_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/projects/{project_id}/branches"))
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
            .unwrap_or_else(|error| panic!("request failed: {error}"));
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
                "projectId": project_id,
                "title": "Ship handlers",
                "status": "todo",
                "workspaceMode": "worktree",
                "type": "default",
                "workflowRunId": null,
            })
        );
        assert_eq!(
            response_json(branch_response).await,
            json!({
                "branches": [
                    {
                        "name": "main",
                        "refName": "main",
                        "displayName": "main",
                    },
                    {
                        "name": format!("ora/{}", &task_id[..8]),
                        "refName": format!("ora/{}", &task_id[..8]),
                        "displayName": "Ship handlers",
                    },
                ],
            })
        );
        assert_eq!(
            response_json(list_response).await,
            json!({
                "tasks": [
                    {
                        "id": task_id,
                        "projectId": project_id,
                        "title": "Ship handlers",
                        "status": "todo",
                        "workspaceMode": "worktree",
                        "type": "default",
                        "workflowRunId": null,
                    },
                ],
            })
        );
        assert_eq!(
            response_json(get_response).await,
            json!({
                "task": {
                    "id": task_id,
                    "projectId": project_id,
                    "title": "Ship handlers",
                    "status": "todo",
                    "workspaceMode": "worktree",
                    "type": "default",
                    "workflowRunId": null,
                },
            })
        );
        assert_eq!(
            response_json(update_response).await,
            json!({
                "task": {
                    "id": task_id,
                    "projectId": project_id,
                    "title": "Ship updated handlers",
                    "status": "doing",
                    "workspaceMode": "worktree",
                    "type": "default",
                    "workflowRunId": null,
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

    /// Verifies Spec catalog, guarded reads, source resolution, and project-wide overrides share one HTTP contract.
    #[tokio::test]
    async fn serves_spec_management_routes() {
        let (temp_dir, _database_path, app) = test_router();
        let project_root = temp_dir.path().join("spec-project");
        let source_root = project_root.join("docs").join("specs");
        std::fs::create_dir_all(&source_root)
            .unwrap_or_else(|error| panic!("failed to create Spec source: {error}"));
        std::fs::write(source_root.join("design.md"), "# Design\n")
            .unwrap_or_else(|error| panic!("failed to write Spec fixture: {error}"));
        let project_id =
            create_project(&app, "Spec project", &project_root.to_string_lossy()).await;
        let target = json!({ "kind": "project", "projectId": project_id });
        let project_root_task = request_json(
            &app,
            Method::POST,
            "/api/tasks",
            json!({
                "projectId": project_id,
                "title": "Read root Specs",
                "status": "todo",
                "workspaceMode": "project_root",
            }),
        )
        .await;
        assert_eq!(project_root_task.status(), StatusCode::OK);
        let project_root_task_id = response_json(project_root_task).await["task"]["id"]
            .as_str()
            .expect("project-root task id")
            .to_string();
        let task_workspace = request_empty(
            &app,
            Method::GET,
            &format!("/api/tasks/{project_root_task_id}/workspace"),
        )
        .await;
        assert_eq!(task_workspace.status(), StatusCode::OK);
        assert_eq!(
            response_json(task_workspace).await,
            json!({
                "workspace": {
                    "rootPath": project_root,
                },
            })
        );

        let catalog = request_json(
            &app,
            Method::POST,
            "/api/specs/catalog",
            json!({ "target": target.clone() }),
        )
        .await;
        assert_eq!(catalog.status(), StatusCode::OK);
        let catalog = response_json(catalog).await;
        assert_eq!(catalog["truncated"], false);
        assert_eq!(
            catalog["documents"][0]["relativePath"],
            "docs/specs/design.md"
        );
        assert_eq!(catalog["documents"][0]["sourceRelativePath"], "docs/specs");

        let task_catalog = request_json(
            &app,
            Method::POST,
            "/api/specs/catalog",
            json!({ "target": { "kind": "task", "taskId": project_root_task_id } }),
        )
        .await;
        assert_eq!(task_catalog.status(), StatusCode::OK);
        assert_eq!(
            response_json(task_catalog).await["documents"][0]["relativePath"],
            "docs/specs/design.md"
        );

        let read = request_json(
            &app,
            Method::POST,
            "/api/specs/read",
            json!({
                "target": target.clone(),
                "relativePath": "docs/specs/design.md",
            }),
        )
        .await;
        assert_eq!(read.status(), StatusCode::OK);
        assert_eq!(
            response_json(read).await,
            json!({
                "relativePath": "docs/specs/design.md",
                "content": "# Design\n",
                "byteSize": 9,
            })
        );

        let unauthorized_read = request_json(
            &app,
            Method::POST,
            "/api/specs/read",
            json!({
                "target": target.clone(),
                "relativePath": "README.md",
            }),
        )
        .await;
        assert_eq!(unauthorized_read.status(), StatusCode::NOT_FOUND);
        assert_contract_error(
            &response_json(unauthorized_read).await,
            "spec_document_not_found",
        );

        let resolved = request_json(
            &app,
            Method::POST,
            "/api/specs/resolve-source",
            json!({
                "target": target.clone(),
                "absolutePath": source_root,
            }),
        )
        .await;
        assert_eq!(resolved.status(), StatusCode::OK);
        assert_eq!(
            response_json(resolved).await,
            json!({
                "relativePath": "docs/specs",
                "workflow": { "kind": "custom", "name": "Custom" },
            })
        );

        let root_source = request_json(
            &app,
            Method::POST,
            "/api/specs/resolve-source",
            json!({
                "target": target.clone(),
                "absolutePath": project_root,
            }),
        )
        .await;
        assert_eq!(root_source.status(), StatusCode::BAD_REQUEST);
        assert_contract_error(
            &response_json(root_source).await,
            "spec_source_workspace_root",
        );

        let update = request_json(
            &app,
            Method::PUT,
            &format!("/api/projects/{project_id}/spec-sources"),
            json!({
                "sources": [{
                    "relativePath": "docs/specs",
                    "workflow": { "kind": "custom", "name": "Custom" },
                    "visibility": "disabled",
                }, {
                    "relativePath": "architecture/missing",
                    "workflow": { "kind": "custom", "name": " Architecture " },
                    "visibility": "enabled",
                }],
            }),
        )
        .await;
        assert_eq!(update.status(), StatusCode::OK);
        assert_eq!(
            response_json(update).await["sources"][0]["visibility"],
            "disabled"
        );

        let disabled_catalog = request_json(
            &app,
            Method::POST,
            "/api/specs/catalog",
            json!({ "target": target }),
        )
        .await;
        assert_eq!(disabled_catalog.status(), StatusCode::OK);
        let disabled_catalog = response_json(disabled_catalog).await;
        assert_eq!(disabled_catalog["documents"], json!([]));
        assert!(
            disabled_catalog["sources"]
                .as_array()
                .is_some_and(|sources| {
                    sources.iter().any(|source| {
                        source["relativePath"] == "architecture/missing"
                            && source["workflow"]
                                == json!({ "kind": "custom", "name": "Architecture" })
                            && source["availability"] == "missing"
                    })
                })
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

    /// Verifies session query routes expose stable empty and not-found responses.
    #[tokio::test]
    async fn serves_session_query_routes() {
        let (_temp_dir, _database_path, app) = test_router();
        let list_response = request_empty(&app, Method::GET, "/api/sessions").await;
        let get_response = request_empty(&app, Method::GET, "/api/sessions/missing-session").await;

        assert_eq!(
            response_json(list_response).await,
            json!({ "sessions": [] })
        );
        assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
        assert_contract_error(&response_json(get_response).await, "session_not_found");
    }

    /// Verifies catalog routes address resources by identifier while names remain editable fields.
    #[tokio::test]
    async fn serves_skill_and_agent_crud_routes() {
        let (_temp_dir, _database_path, app) = test_router();
        let skill_create = request_json(
            &app,
            Method::POST,
            "/api/skills",
            json!({ "name": " review-guide ", "description": "Reviews guides", "content": "# Original skill" }),
        )
        .await;
        assert_eq!(skill_create.status(), StatusCode::OK);
        let skill = response_json(skill_create).await;
        let skill_id = skill["skill"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("response did not include a skill id"))
            .to_string();
        assert_eq!(skill["skill"]["name"], "review-guide");
        let skill_path = format!("/api/skills/{skill_id}");
        let skill_get = request_empty(&app, Method::GET, &skill_path).await;
        assert_eq!(skill_get.status(), StatusCode::OK);
        assert_eq!(
            response_json(skill_get).await["skill"]["content"],
            "# Original skill"
        );
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
        let preserved_skill = request_empty(&app, Method::GET, &skill_path).await;
        assert_eq!(
            response_json(preserved_skill).await["skill"]["content"],
            "# Original skill"
        );
        let skill_content_update = request_json(
            &app,
            Method::PUT,
            &skill_path,
            json!({ "name": "reviewer", "description": "Reviews changes", "content": "# Updated skill" }),
        )
        .await;
        assert_eq!(skill_content_update.status(), StatusCode::OK);
        let updated_skill = request_empty(&app, Method::GET, &skill_path).await;
        assert_eq!(
            response_json(updated_skill).await["skill"]["content"],
            "# Updated skill"
        );
        let duplicate_skill = request_json(
            &app,
            Method::POST,
            "/api/skills",
            json!({ "name": "Reviewer", "description": "Duplicate" }),
        )
        .await;
        assert_eq!(duplicate_skill.status(), StatusCode::CONFLICT);
        assert_contract_error(&response_json(duplicate_skill).await, "skill_name_conflict");
        let invalid_slug = request_json(
            &app,
            Method::POST,
            "/api/skills",
            json!({ "name": "bad/name", "description": "Invalid" }),
        )
        .await;
        assert_eq!(invalid_slug.status(), StatusCode::BAD_REQUEST);
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
            json!({ "name": "opencode", "description": "OpenCode", "content": "# Original agent" }),
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
        let agent_get = request_empty(&app, Method::GET, &agent_path).await;
        assert_eq!(agent_get.status(), StatusCode::OK);
        assert_eq!(
            response_json(agent_get).await["agent"]["content"],
            "# Original agent"
        );
        let agent_update = request_json(
            &app,
            Method::PUT,
            &agent_path,
            json!({ "name": "review-agent", "description": "Reviews changes" }),
        )
        .await;
        assert_eq!(agent_update.status(), StatusCode::OK);
        let preserved_agent = request_empty(&app, Method::GET, &agent_path).await;
        assert_eq!(
            response_json(preserved_agent).await["agent"]["content"],
            "# Original agent"
        );
        let agent_content_update = request_json(
            &app,
            Method::PUT,
            &agent_path,
            json!({ "name": "review-agent", "description": "Reviews changes", "content": "# Updated agent" }),
        )
        .await;
        assert_eq!(agent_content_update.status(), StatusCode::OK);
        let updated_agent = request_empty(&app, Method::GET, &agent_path).await;
        assert_eq!(
            response_json(updated_agent).await["agent"]["content"],
            "# Updated agent"
        );
        let duplicate_agent = request_json(
            &app,
            Method::POST,
            "/api/agents",
            json!({ "name": " Review-Agent ", "description": "Duplicate" }),
        )
        .await;
        assert_eq!(duplicate_agent.status(), StatusCode::CONFLICT);
        assert_contract_error(&response_json(duplicate_agent).await, "agent_name_conflict");
        let second_agent = response_json(
            request_json(
                &app,
                Method::POST,
                "/api/agents",
                json!({ "name": "writer", "description": "Writes changes" }),
            )
            .await,
        )
        .await;
        let second_agent_id = second_agent["agent"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("response did not include a second agent id"));
        let conflicting_rename = request_json(
            &app,
            Method::PUT,
            &format!("/api/agents/{second_agent_id}"),
            json!({ "name": "REVIEW-AGENT", "description": "Duplicate" }),
        )
        .await;
        assert_eq!(conflicting_rename.status(), StatusCode::CONFLICT);
        assert_contract_error(
            &response_json(conflicting_rename).await,
            "agent_name_conflict",
        );
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

    /// Verifies the router supports workflow definition CRUD, publish, versions, and snapshot-by-id.
    #[tokio::test]
    async fn serves_workflow_definition_routes() {
        let (_temp_dir, _database_path, app) = test_router();

        let create_response = request_json(
            &app,
            Method::POST,
            "/api/workflows",
            json!({
                "name": "Review flow",
                "graph": "{\"nodes\":[]}",
            }),
        )
        .await;
        assert_eq!(create_response.status(), StatusCode::OK);
        let workflow_id = response_json(create_response).await["workflow"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("response did not include a workflow id"))
            .to_string();
        let workflow_path = format!("/api/workflows/{workflow_id}");

        let list_response = request_empty(&app, Method::GET, "/api/workflows").await;
        assert_eq!(list_response.status(), StatusCode::OK);
        assert_eq!(
            response_json(list_response).await["workflows"][0]["name"],
            json!("Review flow")
        );

        let get_response = request_empty(&app, Method::GET, &workflow_path).await;
        assert_eq!(get_response.status(), StatusCode::OK);
        assert_eq!(
            response_json(get_response).await["workflow"]["name"],
            json!("Review flow")
        );

        let draft_path = format!("{workflow_path}/draft");
        let update_draft = request_json(
            &app,
            Method::PUT,
            &draft_path,
            json!({ "graph": "{\"nodes\":[{\"id\":\"start\"}]}" }),
        )
        .await;
        assert_eq!(update_draft.status(), StatusCode::OK);
        let get_draft = request_empty(&app, Method::GET, &draft_path).await;
        assert_eq!(get_draft.status(), StatusCode::OK);
        assert_eq!(
            response_json(get_draft).await["snapshot"]["graph"],
            json!("{\"nodes\":[{\"id\":\"start\"}]}")
        );

        let publish_response = request_json(
            &app,
            Method::POST,
            &format!("{workflow_path}/publish"),
            json!({}),
        )
        .await;
        assert_eq!(publish_response.status(), StatusCode::OK);
        let published = response_json(publish_response).await;
        let snapshot_id = published["snapshot"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("publish response did not include a snapshot id"))
            .to_string();
        let version = published["snapshot"]["version"]
            .as_str()
            .unwrap_or_else(|| panic!("publish response did not include a version"))
            .to_string();

        let versions_response =
            request_empty(&app, Method::GET, &format!("{workflow_path}/versions")).await;
        assert_eq!(versions_response.status(), StatusCode::OK);
        assert_eq!(
            response_json(versions_response).await["versions"][0]["version"],
            json!(version)
        );

        let snapshot_response = request_empty(
            &app,
            Method::GET,
            &format!("/api/workflow-snapshots/{snapshot_id}"),
        )
        .await;
        assert_eq!(snapshot_response.status(), StatusCode::OK);
        assert_eq!(
            response_json(snapshot_response).await["snapshot"]["id"],
            json!(snapshot_id)
        );
    }

    /// Verifies the router supports workflow-run create, list scopes, get, and delete.
    #[tokio::test]
    async fn serves_workflow_run_routes() {
        let (temp_dir, _database_path, app) = test_router();
        let project_root = temp_dir.path().join("repo").to_string_lossy().to_string();
        let project_id = create_project(&app, "Run project", &project_root).await;

        let create_workflow = request_json(
            &app,
            Method::POST,
            "/api/workflows",
            json!({ "name": "Run flow", "graph": "{\"nodes\":[],\"edges\":[]}" }),
        )
        .await;
        let workflow_id = response_json(create_workflow).await["workflow"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("create workflow response did not include an id"))
            .to_string();
        let publish_response = request_json(
            &app,
            Method::POST,
            &format!("/api/workflows/{workflow_id}/publish"),
            json!({}),
        )
        .await;
        assert_eq!(publish_response.status(), StatusCode::OK);

        let create_run = request_json(
            &app,
            Method::POST,
            "/api/workflow-runs",
            json!({
                "projectId": project_id,
                "workflowId": workflow_id,
                "name": "First run",
            }),
        )
        .await;
        assert_eq!(create_run.status(), StatusCode::OK);
        let run_id = response_json(create_run).await["run"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("create run response did not include an id"))
            .to_string();

        let by_project = request_empty(
            &app,
            Method::GET,
            &format!("/api/workflow-runs?projectId={project_id}"),
        )
        .await;
        assert_eq!(
            response_json(by_project).await["runs"][0]["id"],
            json!(run_id)
        );
        let by_workflow = request_empty(
            &app,
            Method::GET,
            &format!("/api/workflow-runs?workflowId={workflow_id}"),
        )
        .await;
        assert_eq!(
            response_json(by_workflow).await["runs"][0]["id"],
            json!(run_id)
        );

        let run_path = format!("/api/workflow-runs/{run_id}");
        let get_run = request_empty(&app, Method::GET, &run_path).await;
        assert_eq!(get_run.status(), StatusCode::OK);
        let run_detail = response_json(get_run).await;
        assert_eq!(run_detail["name"], json!("First run"));
        assert!(run_detail["taskId"].as_str().is_some());

        let delete_run = request_empty(&app, Method::DELETE, &run_path).await;
        assert_eq!(delete_run.status(), StatusCode::OK);
        assert_eq!(response_json(delete_run).await["runId"], json!(run_id));
    }

    /// Verifies one Markdown document is sent as JSON, imported, read back, and conflict-skipped.
    #[tokio::test]
    async fn imports_agent_markdown_through_json_routes() {
        let (_temp_dir, _database_path, app) = test_router();
        let markdown =
            "---\nname: review-agent\ndescription: Reviews changes\n---\nReview changes.\n";

        let prepare = request_json(
            &app,
            Method::POST,
            "/api/agent-imports/prepare",
            json!({ "content": markdown }),
        )
        .await;
        assert_eq!(prepare.status(), StatusCode::OK);
        let candidate = response_json(prepare).await;
        assert_eq!(candidate["candidate"]["status"], "ready");
        assert_eq!(candidate["candidate"]["name"], "review-agent");
        assert_eq!(candidate["candidate"]["existingAgent"], Value::Null);

        let commit = request_json(
            &app,
            Method::POST,
            "/api/agent-imports/commit",
            json!({
                "content": markdown,
                "decision": null,
                "expectedAgentId": null,
                "expectedUpdatedAt": null,
            }),
        )
        .await;
        assert_eq!(commit.status(), StatusCode::OK);
        let imported = response_json(commit).await;
        assert_eq!(imported["status"], "imported");
        let agent_id = imported["agent"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("import response did not include an agent id"));
        let details = response_json(
            request_empty(&app, Method::GET, &format!("/api/agents/{agent_id}")).await,
        )
        .await;
        assert_eq!(details["agent"]["content"], "Review changes.");

        let conflict = response_json(
            request_json(
                &app,
                Method::POST,
                "/api/agent-imports/prepare",
                json!({ "content": markdown }),
            )
            .await,
        )
        .await;
        assert_eq!(conflict["candidate"]["status"], "conflict");
        assert_eq!(conflict["candidate"]["existingAgent"]["agentId"], agent_id);
        let skipped = response_json(
            request_json(
                &app,
                Method::POST,
                "/api/agent-imports/commit",
                json!({
                    "content": markdown,
                    "decision": "skip",
                    "expectedAgentId": agent_id,
                    "expectedUpdatedAt": conflict["candidate"]["existingAgent"]["updatedAt"],
                }),
            )
            .await,
        )
        .await;
        assert_eq!(skipped, json!({ "status": "skipped", "agent": null }));

        let invalid = request_json(
            &app,
            Method::POST,
            "/api/agent-imports/prepare",
            json!({ "content": "# Missing frontmatter" }),
        )
        .await;
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
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
        let app_state =
            build_app_state_for_database(&database_path, &project_root, &work_dir, temp_dir.path())
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

    fn assert_contract_error(body: &Value, expected_code: &str) {
        assert_eq!(
            body.get("code").and_then(Value::as_str),
            Some(expected_code)
        );
        assert!(body.get("params").is_some_and(Value::is_object));
        let request_id = body
            .get("requestId")
            .cloned()
            .expect("contract error must include requestId");
        serde_json::from_value::<RequestId>(request_id)
            .expect("contract error requestId must be a UUID");
        assert!(body.get("message").is_none());
        assert!(body.get("error").is_none());
    }
}
