use std::path::{Path, PathBuf};
use std::sync::Arc;

#[path = "scheduled_task_artifact.rs"]
mod artifact;
#[path = "scheduled_task_calendar.rs"]
mod calendar;
#[path = "scheduled_task_runtime.rs"]
mod runtime;
#[path = "scheduled_task_worktree.rs"]
mod worktree;

use ora_application::{
    CopyScheduledRunArtifactHandler, CreateScheduledTaskHandler, DeleteScheduledTaskHandler,
    GetScheduledTaskHandler, GitScheduledTaskTargetValidator, ListScheduledRunArtifactsHandler,
    ListScheduledTaskHistoryHandler, ListScheduledTasksHandler, ReadScheduledRunArtifactHandler,
    RevokeScheduledTaskApprovalHandler, ScheduledArtifactStore, UpdateScheduledTaskHandler,
    UuidScheduledTaskIdGenerator,
};
use ora_contracts::{
    AgentModel, CreateScheduledTaskRequest, CreateScheduledTaskResponse,
    DeleteScheduledTaskRequest, DeleteScheduledTaskResponse, EmptyErrorParams,
    GetScheduledTaskRequest, GetScheduledTaskResponse, ListAgentModelsRequest,
    ListScheduledRunArtifactsRequest, ListScheduledRunArtifactsResponse,
    ListScheduledTaskHistoryRequest, ListScheduledTaskHistoryResponse, ListScheduledTasksRequest,
    ListScheduledTasksResponse, PublicError, ReadScheduledRunArtifactRequest,
    ReadScheduledRunArtifactResponse, RevokeScheduledTaskApprovalRequest,
    RevokeScheduledTaskApprovalResponse, ScheduledRunArtifactInfo,
    ScheduledRunArtifactRejectionInfo, UpdateScheduledTaskRequest, UpdateScheduledTaskResponse,
};
use ora_db::{
    RepositoryPool, SqliteProjectRepository, SqliteScheduledRunRepository,
    SqliteScheduledTaskRepository, SqliteWorkspaceRepository,
};
use ora_domain::ScheduledRunId;
use ora_domain::ScheduledTaskExecutionLocation;
use ora_scheduler::Scheduler;

use crate::{
    AgentRuntime, BackendError, ErrorClassification, Sessions, TaskApi, clock::SystemClock,
};

use runtime::ScheduledTaskRuntime;

/// Caps the text returned to the history preview so one 10 MiB artifact cannot stall IPC.
const MAX_SCHEDULED_ARTIFACT_PREVIEW_BYTES: usize = 512 * 1024;

/// Carries the shared services and storage root needed to compose the Scheduled Task API.
pub(crate) struct ScheduledTaskSetup {
    pub pool: RepositoryPool,
    pub agent_runtime: AgentRuntime,
    pub scheduler: Scheduler,
    pub tasks_api: Arc<TaskApi>,
    pub sessions: Arc<Sessions>,
    pub clock: SystemClock,
    pub events: crate::app_event::AppEventPublisher,
    pub artifacts_root: PathBuf,
}

/// Groups concrete Scheduled Task definition handlers shared by runtime adapters.
pub struct ScheduledTaskApi {
    agent_runtime: AgentRuntime,
    workspaces: SqliteWorkspaceRepository,
    create: CreateScheduledTaskHandler<
        SqliteScheduledTaskRepository,
        SqliteProjectRepository,
        SqliteWorkspaceRepository,
        GitScheduledTaskTargetValidator,
        UuidScheduledTaskIdGenerator,
        SystemClock,
    >,
    get: GetScheduledTaskHandler<SqliteScheduledTaskRepository>,
    list: ListScheduledTasksHandler<SqliteScheduledTaskRepository>,
    list_history: ListScheduledTaskHistoryHandler<SqliteScheduledRunRepository>,
    update: UpdateScheduledTaskHandler<
        SqliteScheduledTaskRepository,
        SqliteProjectRepository,
        SqliteWorkspaceRepository,
        GitScheduledTaskTargetValidator,
        SystemClock,
    >,
    delete: DeleteScheduledTaskHandler<SqliteScheduledTaskRepository, SystemClock>,
    revoke_approval: RevokeScheduledTaskApprovalHandler<SqliteScheduledTaskRepository, SystemClock>,
    list_run_artifacts: ListScheduledRunArtifactsHandler<SqliteScheduledRunRepository>,
    read_run_artifact: ReadScheduledRunArtifactHandler<SqliteScheduledRunRepository>,
    copy_run_artifact: CopyScheduledRunArtifactHandler<SqliteScheduledRunRepository>,
    runtime: Arc<ScheduledTaskRuntime>,
}

impl ScheduledTaskApi {
    /// Builds definition handlers from the shared repository pool.
    pub(crate) fn new(setup: ScheduledTaskSetup) -> Self {
        let ScheduledTaskSetup {
            pool,
            agent_runtime,
            scheduler,
            tasks_api,
            sessions,
            clock,
            events,
            artifacts_root,
        } = setup;
        let repository = Arc::new(SqliteScheduledTaskRepository::new(pool.clone()));
        let projects = Arc::new(SqliteProjectRepository::new(pool.clone()));
        let workspaces = Arc::new(SqliteWorkspaceRepository::new(pool.clone()));
        let run_repository = Arc::new(SqliteScheduledRunRepository::new(pool.clone()));
        let artifact_store = ScheduledArtifactStore::new(artifacts_root);
        let runtime = ScheduledTaskRuntime::new(
            pool,
            scheduler,
            tasks_api,
            sessions,
            clock,
            events,
            artifact_store.clone(),
        );
        Self {
            agent_runtime,
            workspaces: workspaces.as_ref().clone(),
            create: CreateScheduledTaskHandler::new(
                repository.clone(),
                projects.clone(),
                workspaces.clone(),
                GitScheduledTaskTargetValidator,
                UuidScheduledTaskIdGenerator::new(),
                clock,
            ),
            get: GetScheduledTaskHandler::new(repository.clone()),
            list: ListScheduledTasksHandler::new(repository.clone()),
            list_history: ListScheduledTaskHistoryHandler::new(run_repository.clone()),
            list_run_artifacts: ListScheduledRunArtifactsHandler::new(run_repository.clone()),
            read_run_artifact: ReadScheduledRunArtifactHandler::new(
                run_repository.clone(),
                artifact_store.clone(),
            ),
            copy_run_artifact: CopyScheduledRunArtifactHandler::new(run_repository, artifact_store),
            update: UpdateScheduledTaskHandler::new(
                repository.clone(),
                projects,
                workspaces,
                GitScheduledTaskTargetValidator,
                clock,
            ),
            delete: DeleteScheduledTaskHandler::new(repository.clone(), clock),
            revoke_approval: RevokeScheduledTaskApprovalHandler::new(repository, clock),
            runtime,
        }
    }

    /// Creates a persisted Scheduled Task definition.
    pub async fn create(
        &self,
        request: CreateScheduledTaskRequest,
    ) -> Result<CreateScheduledTaskResponse, BackendError> {
        let task = self.create.prepare(request)?;
        if task.enabled && task.execution_location == ScheduledTaskExecutionLocation::Local {
            self.validate_agent_model(
                task.project_id.as_ref(),
                task.agent_ref.as_str(),
                task.model.as_deref(),
            )
            .await?;
        }
        let task_for_runtime = task.clone();
        let response = self.create.persist(task).map_err(BackendError::from)?;
        self.runtime.reconcile(task_for_runtime);
        Ok(response)
    }

    /// Loads one Scheduled Task definition.
    pub fn get(
        &self,
        request: GetScheduledTaskRequest,
    ) -> Result<GetScheduledTaskResponse, BackendError> {
        self.get.handle(request).map_err(BackendError::from)
    }

    /// Lists non-deleted Scheduled Task definitions.
    pub fn list(
        &self,
        request: ListScheduledTasksRequest,
    ) -> Result<ListScheduledTasksResponse, BackendError> {
        self.list.handle(request).map_err(BackendError::from)
    }

    /// Lists persisted run and missed facts for one Scheduled Task.
    pub fn list_history(
        &self,
        request: ListScheduledTaskHistoryRequest,
    ) -> Result<ListScheduledTaskHistoryResponse, BackendError> {
        self.list_history
            .handle(request)
            .map_err(BackendError::from)
    }

    /// Lists one run's collected artifacts and rejected candidates.
    pub fn list_run_artifacts(
        &self,
        request: ListScheduledRunArtifactsRequest,
    ) -> Result<ListScheduledRunArtifactsResponse, BackendError> {
        let run_id = ScheduledRunId::new(request.run_id);
        let (artifacts, rejections) = self
            .list_run_artifacts
            .handle(&run_id)
            .map_err(BackendError::from)?;
        Ok(ListScheduledRunArtifactsResponse {
            artifacts: artifacts.into_iter().map(contract_artifact).collect(),
            rejections: rejections
                .into_iter()
                .map(contract_artifact_rejection)
                .collect(),
        })
    }

    /// Returns a bounded text preview of one registered artifact.
    pub fn read_run_artifact(
        &self,
        request: ReadScheduledRunArtifactRequest,
    ) -> Result<ReadScheduledRunArtifactResponse, BackendError> {
        let run_id = ScheduledRunId::new(request.run_id);
        let bytes = self
            .read_run_artifact
            .handle(&run_id, &request.relative_path)
            .map_err(BackendError::from)?;
        let truncated = bytes.len() > MAX_SCHEDULED_ARTIFACT_PREVIEW_BYTES;
        let preview = &bytes[..bytes.len().min(MAX_SCHEDULED_ARTIFACT_PREVIEW_BYTES)];
        Ok(ReadScheduledRunArtifactResponse {
            content: String::from_utf8_lossy(preview).into_owned(),
            truncated,
        })
    }

    /// Copies one registered artifact to the native save dialog's destination.
    pub fn copy_run_artifact(
        &self,
        run_id: &str,
        relative_path: &str,
        destination: &Path,
    ) -> Result<(), BackendError> {
        self.copy_run_artifact
            .handle(&ScheduledRunId::new(run_id), relative_path, destination)
            .map_err(BackendError::from)
    }

    /// Updates mutable Scheduled Task definition fields.
    pub async fn update(
        &self,
        request: UpdateScheduledTaskRequest,
    ) -> Result<UpdateScheduledTaskResponse, BackendError> {
        let prepared = self.update.prepare(request)?;
        let task = prepared.task();
        if task.enabled && task.execution_location == ScheduledTaskExecutionLocation::Local {
            self.validate_agent_model(
                task.project_id.as_ref(),
                task.agent_ref.as_str(),
                task.model.as_deref(),
            )
            .await?;
        }
        let task_for_runtime = prepared.task().clone();
        let response = self.update.persist(prepared).map_err(BackendError::from)?;
        self.runtime.reconcile(task_for_runtime);
        Ok(response)
    }

    async fn validate_agent_model(
        &self,
        project_id: &str,
        agent_ref: &str,
        model: Option<&str>,
    ) -> Result<(), BackendError> {
        let workspace = self
            .workspaces
            .find_main_workspace(&ora_domain::ProjectId::new(project_id))
            .map_err(|source| {
                BackendError::internal("scheduled task workspace lookup failed", source)
            })?
            .ok_or_else(|| {
                invalid_runtime_selection("scheduled task project has no main workspace")
            })?;
        let response = self
            .agent_runtime
            .models(ListAgentModelsRequest {
                agent_ref: agent_ref.to_string(),
                workspace_id: workspace.id.to_string(),
            })
            .await?;
        require_requested_model(model, &response.models)
    }

    /// Soft-deletes a Scheduled Task definition.
    pub fn delete(
        &self,
        request: DeleteScheduledTaskRequest,
    ) -> Result<DeleteScheduledTaskResponse, BackendError> {
        let response = self
            .delete
            .handle(request.clone())
            .map_err(BackendError::from)?;
        self.runtime
            .cancel(&ora_domain::ScheduledTaskId::new(request.task_id));
        Ok(response)
    }

    /// Revokes local-background approval and disables the task.
    pub fn revoke_approval(
        &self,
        request: RevokeScheduledTaskApprovalRequest,
    ) -> Result<RevokeScheduledTaskApprovalResponse, BackendError> {
        let task_id = request.task_id.clone();
        let response = self
            .revoke_approval
            .handle(request)
            .map_err(BackendError::from)?;
        self.runtime
            .cancel(&ora_domain::ScheduledTaskId::new(task_id));
        Ok(response)
    }
}

fn require_requested_model(model: Option<&str>, models: &[AgentModel]) -> Result<(), BackendError> {
    let Some(model) = model else {
        return Ok(());
    };
    if models.iter().any(|candidate| candidate.id == model) {
        return Ok(());
    }
    Err(invalid_runtime_selection(
        "scheduled task model is unavailable for the selected agent",
    ))
}

/// Maps one domain artifact into its public contract shape.
fn contract_artifact(artifact: ora_domain::ScheduledRunArtifact) -> ScheduledRunArtifactInfo {
    ScheduledRunArtifactInfo {
        relative_path: artifact.relative_path,
        kind: match artifact.kind {
            ora_domain::ScheduledArtifactKind::Markdown => {
                ora_contracts::ScheduledArtifactKind::Markdown
            }
            ora_domain::ScheduledArtifactKind::Json => ora_contracts::ScheduledArtifactKind::Json,
            ora_domain::ScheduledArtifactKind::Text => ora_contracts::ScheduledArtifactKind::Text,
            ora_domain::ScheduledArtifactKind::Log => ora_contracts::ScheduledArtifactKind::Log,
            ora_domain::ScheduledArtifactKind::Xml => ora_contracts::ScheduledArtifactKind::Xml,
            ora_domain::ScheduledArtifactKind::Html => ora_contracts::ScheduledArtifactKind::Html,
        },
        size_bytes: artifact.size_bytes,
        registered_at: artifact.registered_at,
    }
}

/// Maps one domain rejection into its public contract shape.
fn contract_artifact_rejection(
    rejection: ora_domain::ScheduledArtifactRejection,
) -> ScheduledRunArtifactRejectionInfo {
    use ora_contracts::ScheduledArtifactRejectionReason as Contract;
    use ora_domain::ScheduledArtifactRejectionReason as Domain;
    ScheduledRunArtifactRejectionInfo {
        name: rejection.name,
        reason: match rejection.reason {
            Domain::UnsupportedType => Contract::UnsupportedType,
            Domain::PathUnsafe => Contract::PathUnsafe,
            Domain::Symlink => Contract::Symlink,
            Domain::HardLink => Contract::HardLink,
            Domain::TooLarge => Contract::TooLarge,
            Domain::TooMany => Contract::TooMany,
            Domain::TotalTooLarge => Contract::TotalTooLarge,
            Domain::ReadFailed => Contract::ReadFailed,
            Domain::CopyFailed => Contract::CopyFailed,
        },
    }
}

fn invalid_runtime_selection(context: &'static str) -> BackendError {
    BackendError::new(
        ErrorClassification::InvalidRequest,
        PublicError::InvalidRequest(EmptyErrorParams {}),
        context,
    )
}

#[cfg(test)]
mod tests {
    use super::require_requested_model;
    use crate::Backend;
    use crate::test_backend::backend_paths;
    use ora_contracts::{
        AgentModel, CreateProjectRequest, CreateScheduledTaskRequest, EmptyErrorParams,
        PublicError, ScheduledTaskApprovalChange, ScheduledTaskApprovalPolicy,
        ScheduledTaskExecutionLocation, ScheduledTaskSchedule, UpdateScheduledTaskRequest,
    };
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    fn model(id: &str) -> AgentModel {
        AgentModel {
            id: id.to_string(),
            display_name: id.to_string(),
            default: false,
        }
    }

    /// Verifies a project added as a plain folder refuses even a disabled draft with the typed
    /// Git requirement. A Desktop project never persists a default branch, so creation always
    /// probes the folder for one; on a plain folder Git fails, which must not surface as an
    /// internal error.
    #[tokio::test]
    async fn rejects_a_disabled_draft_for_a_plain_folder_project_as_invalid() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let folder = temporary.path().join("plain");
        std::fs::create_dir_all(&folder).expect("create plain project folder");
        let backend =
            Backend::open(backend_paths(temporary.path(), temporary.path())).expect("open backend");
        let project_id = backend
            .projects()
            .create(CreateProjectRequest {
                name: "Plain".to_string(),
                main_workspace_path: folder.to_string_lossy().into_owned(),
            })
            .expect("create plain folder project")
            .project
            .id;

        let error = backend
            .scheduled_tasks()
            .create(CreateScheduledTaskRequest {
                name: "Nightly notes".to_string(),
                enabled: false,
                schedule: ScheduledTaskSchedule::EveryHours { interval: 24 },
                execution_location: ScheduledTaskExecutionLocation::Local,
                project_id,
                agent_ref: "ora/claude-code".to_string(),
                model: None,
                prompt: "Summarize the folder".to_string(),
                approval_policy: None,
            })
            .await
            .expect_err("a plain folder cannot host a scheduled task");

        assert_eq!(
            error.public_error().clone(),
            PublicError::InvalidRequest(EmptyErrorParams {})
        );
    }

    /// Verifies enabling an approved task whose project folder lost its repository (the list
    /// switch path) is refused with the typed Git requirement before any runtime check.
    #[tokio::test]
    async fn rejects_enabling_a_task_whose_project_folder_is_no_longer_git() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let scaffold = ora_test_support::GitTestScaffold::new("backend-scheduled-enable-plain")
            .expect("create Git test scaffold");
        scaffold
            .write_file(scaffold.repo_path(), "README.md", "seed\n")
            .expect("write seed file");
        scaffold
            .stage_all_and_commit("initial")
            .expect("create seed commit");
        let backend =
            Backend::open(backend_paths(temporary.path(), temporary.path())).expect("open backend");
        let project_id = backend
            .projects()
            .create(CreateProjectRequest {
                name: "Repository".to_string(),
                main_workspace_path: scaffold.repo_path().to_string_lossy().into_owned(),
            })
            .expect("create Git project")
            .project
            .id;
        let task = backend
            .scheduled_tasks()
            .create(CreateScheduledTaskRequest {
                name: "Nightly notes".to_string(),
                enabled: false,
                schedule: ScheduledTaskSchedule::EveryHours { interval: 24 },
                execution_location: ScheduledTaskExecutionLocation::Local,
                project_id: project_id.clone(),
                agent_ref: "ora/claude-code".to_string(),
                model: None,
                prompt: "Summarize the repository".to_string(),
                approval_policy: Some(ScheduledTaskApprovalPolicy::FullAutoInWorktree),
            })
            .await
            .expect("create approved disabled task")
            .task;
        std::fs::remove_dir_all(scaffold.repo_path().join(".git")).expect("remove repository");

        let error = backend
            .scheduled_tasks()
            .update(UpdateScheduledTaskRequest {
                task_id: task.id,
                expected_updated_at: task.updated_at,
                name: task.name,
                enabled: true,
                schedule: task.schedule,
                project_id,
                agent_ref: task.agent_ref,
                model: task.model,
                prompt: task.prompt,
                approval_change: ScheduledTaskApprovalChange::Keep,
            })
            .await
            .expect_err("a plain folder cannot run a scheduled task");

        assert_eq!(
            error.public_error().clone(),
            PublicError::InvalidRequest(EmptyErrorParams {})
        );
    }

    #[test]
    fn accepts_the_runtime_default_or_an_advertised_model() {
        let models = vec![model("fast"), model("smart")];

        assert!(require_requested_model(None, &models).is_ok());
        assert!(require_requested_model(Some("smart"), &models).is_ok());
    }

    #[test]
    fn rejects_a_model_the_selected_agent_does_not_advertise() {
        let error = require_requested_model(Some("retired"), &[model("smart")])
            .expect_err("reject missing model");

        assert_eq!(
            error.classification(),
            crate::ErrorClassification::InvalidRequest
        );
    }
}
