use super::checkpoint::{fail_dispatched_node, record_pre_node_checkpoint};
use super::last_failure::previous_failure_for_injection;
use super::prompt::{
    RequiredWorkflowSkill, WorkflowPromptRequest, assemble_workflow_prompt, render_previous_failure,
};
use crate::agent_runtime::AgentRuntimeManager;
use crate::clock::SystemClock;
use crate::error::BackendError;
use agent_client_protocol_schema::v1::StopReason;
use agent_client_protocol_schema::v1::{
    SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
    SessionConfigSelectOptions,
};
use ora_application::{
    AgentDefinitionRepository, AgentOutputContract, AgentSkill, BindWorkflowNodeSessionResult,
    Clock, ExecutionContext, FileChange, NodeExecutor, NodeFailure, NodeFailureKind,
    RepositoryError, StructuredOutputError, VariableTemplateError, WorkflowGraphNode,
    WorkflowRunCallback, WorkflowRunEngineRepository, WorkflowRunPayload, extract_json_object,
    render_variable_template, validate_against_schema,
};
use ora_contracts::{
    AgentRef as ContractAgentRef, PromptSessionEvent, PromptSessionRequest, StartSessionRequest,
    StopSessionRequest,
};
use ora_db::{RepositoryPool, SqliteAgentDefinitionRepository, SqliteWorkflowRunEngineRepository};
use ora_domain::{AgentDefinitionId, Namespace, SessionId, WorkflowNodeRunId, WorkflowNodeStatus};
use ora_logging::ora_warn;

use super::transitions::WorkflowRunTransitions;
use super::worktree::{capture_worktree_snapshot, compute_file_changes, persist_worktree_baseline};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use std::sync::Arc;
use thiserror::Error;

mod output;
mod payload;
pub(super) use output::AssistantOutputAccumulator;
use payload::{parse_workflow_run_payload, report_outcome};

/// Executes one agent node through a real Ora session, reporting completion to the run engine.
///
/// `dispatch` spawns a background task that starts, prompts, and stops one
/// dedicated session per node, then reports the result through the `WorkflowRunCallback`.
#[derive(Clone)]
pub struct WorkflowRunNodeExecutor {
    agent_runtime: Arc<AgentRuntimeManager>,
    pool: RepositoryPool,
    agent_repository: SqliteAgentDefinitionRepository,
    callback: Arc<dyn WorkflowRunCallback>,
    clock: SystemClock,
    /// Root for the per-node worktree baseline snapshots an interactive node diffs at completion.
    baselines_root: PathBuf,
    /// Commits the interactive park transition with the shared publish-after-commit discipline.
    transitions: Arc<WorkflowRunTransitions>,
}

impl WorkflowRunNodeExecutor {
    /// Builds an executor from the session runtime, persistence, role catalog, engine callback,
    /// and the shared transition sink.
    pub fn new(
        agent_runtime: Arc<AgentRuntimeManager>,
        pool: RepositoryPool,
        agent_repository: SqliteAgentDefinitionRepository,
        callback: Arc<dyn WorkflowRunCallback>,
        clock: SystemClock,
        baselines_root: PathBuf,
        transitions: Arc<WorkflowRunTransitions>,
    ) -> Self {
        Self {
            agent_runtime,
            pool,
            agent_repository,
            callback,
            clock,
            baselines_root,
            transitions,
        }
    }
}

impl NodeExecutor for WorkflowRunNodeExecutor {
    fn dispatch(
        &self,
        node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        graph: &ora_application::WorkflowGraph,
        context: &ExecutionContext,
        scope_id: &ora_domain::WorkflowScopeId,
        variable_pool: &ora_application::WorkflowVariablePool,
    ) {
        let agent_runtime = self.agent_runtime.clone();
        let pool = self.pool.clone();
        let agent_repository = self.agent_repository.clone();
        let callback = self.callback.clone();
        let clock = self.clock;
        let baselines_root = self.baselines_root.clone();
        let transitions = self.transitions.clone();
        let node_run_id = node_run_id.clone();
        let node = node.clone();
        let graph = graph.clone();
        let context = context.clone();
        let scope_id = scope_id.clone();
        let variable_pool = variable_pool.clone();
        tokio::spawn(async move {
            match drive_agent_node(
                &agent_runtime,
                &pool,
                &agent_repository,
                &clock,
                &baselines_root,
                &transitions,
                &node_run_id,
                &node,
                &graph,
                &context,
                &scope_id,
                &variable_pool,
            )
            .await
            {
                Ok(outcome) => {
                    // The callback enters the per-run blocking lock and rusqlite, so it must run
                    // on the blocking pool rather than this tokio worker.
                    let callback = callback.clone();
                    let run_id = context.run.id.clone();
                    let node_run_id = node_run_id.clone();
                    let join = tokio::task::spawn_blocking(move || {
                        report_outcome(&callback, &run_id, &node_run_id, outcome);
                    });
                    if let Err(source) = join.await {
                        ora_warn!("workflow node completion callback panicked: {source}");
                    }
                }
                Err(error) => {
                    fail_dispatched_node(
                        callback.clone(),
                        pool.clone(),
                        &agent_runtime,
                        &context.workspace.id,
                        context.run.id.clone(),
                        node_run_id.clone(),
                        error.into_failure_report(),
                    )
                    .await;
                }
            }
        });
    }

    fn on_composite_node_started(
        &self,
        node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        context: &ExecutionContext,
    ) {
        let workspace_root = match self.agent_runtime.workspace_cwd(&context.workspace.id) {
            Ok(root) => root,
            Err(error) => {
                ora_warn!(
                    error = %error,
                    workspace_id = %context.workspace.id,
                    node_id = %node.id,
                    "failed to resolve workspace cwd for composite checkpoint"
                );
                return;
            }
        };
        let repository = SqliteWorkflowRunEngineRepository::new(self.pool.clone());
        if let Err(error) = record_pre_node_checkpoint(
            &repository,
            &workspace_root,
            &context.run.id,
            &node.id,
            node_run_id,
            context.run.snapshot_id.as_ref(),
            self.clock.now_timestamp_millis(),
        ) {
            ora_warn!(
                error = %error,
                run_id = %context.run.id,
                node_id = %node.id,
                node_run_id = %node_run_id,
                "failed to record composite pre-node checkpoint"
            );
        }
    }
}

/// The result of one driven agent node turn.
pub(super) enum AgentNodeOutcome {
    /// The node finished and reports completion or failure through the callback.
    Completed {
        output: Option<String>,
        structured_output: Option<serde_json::Value>,
        stop_reason: StopReason,
        file_changes: Vec<FileChange>,
    },
    /// An interactive node's first turn ended naturally; it parks at `Pending` awaiting input.
    AwaitingInput,
}

/// Failures raised while driving one agent node's session.
#[derive(Debug, Error)]
pub enum NodeExecutionError {
    #[error("agent node names no agent")]
    MissingAgentRef,
    #[error("model {model_id} is not advertised by agent {agent_ref}")]
    WorkflowModelNotFound { agent_ref: String, model_id: String },
    #[error("agent node {node_id} has no agent configuration")]
    MissingAgentConfig { node_id: String },
    #[error("workflow run has invalid frozen execution metadata")]
    InvalidRunPayload,
    #[error("agent node {node_id} prompt template cannot be rendered: {source}")]
    PromptTemplate {
        node_id: String,
        #[source]
        source: VariableTemplateError,
    },
    #[error("agent node {node_id} structured output failed: {source}")]
    StructuredOutput {
        node_id: String,
        #[source]
        source: StructuredOutputError,
        /// Retained so a schema failure does not erase the Agent's auditable final response.
        output: Option<String>,
    },
    #[error("node {node_id} is missing the frozen materialization receipt for skill {skill_id}")]
    MissingSkillMaterialization { node_id: String, skill_id: String },
    #[error("prompt session ended without a stop reason")]
    SessionEndedWithoutStopReason,
    #[error("workflow node stopped before its session became visible")]
    SessionBindingRejected,
    #[error("failed to persist worktree baseline for node {node_id}: {source}")]
    BaselinePersist {
        node_id: String,
        #[source]
        source: std::io::Error,
    },
    #[error("workflow run repository operation failed")]
    Repository(#[from] RepositoryError),
    #[error("session failed: {0}")]
    Session(#[from] BackendError),
}

impl NodeExecutionError {
    /// Classifies a terminal failure into the structured report persisted on the node run.
    fn into_failure_report(self) -> NodeFailure {
        let mut source_chain = Vec::new();
        let mut current = std::error::Error::source(&self);
        while let Some(source) = current {
            source_chain.push(source.to_string());
            current = source.source();
        }
        let kind = match &self {
            Self::MissingAgentRef => NodeFailureKind::MissingAgentRef,
            Self::WorkflowModelNotFound { .. } => NodeFailureKind::WorkflowModelNotFound,
            Self::MissingAgentConfig { .. } => NodeFailureKind::MissingAgentConfig,
            Self::InvalidRunPayload => NodeFailureKind::InvalidRunPayload,
            Self::PromptTemplate { .. } => NodeFailureKind::PromptTemplate,
            Self::StructuredOutput { .. } => NodeFailureKind::StructuredOutput,
            Self::MissingSkillMaterialization { .. } => {
                NodeFailureKind::MissingSkillMaterialization
            }
            Self::SessionEndedWithoutStopReason => NodeFailureKind::SessionEndedWithoutStopReason,
            Self::SessionBindingRejected => NodeFailureKind::SessionBindingRejected,
            Self::BaselinePersist { .. } => NodeFailureKind::BaselinePersist,
            Self::Repository(_) => NodeFailureKind::Repository,
            Self::Session(_) => NodeFailureKind::Session,
        };
        let output = match &self {
            Self::StructuredOutput { output, .. } => output.clone(),
            Self::MissingAgentRef
            | Self::WorkflowModelNotFound { .. }
            | Self::MissingAgentConfig { .. }
            | Self::InvalidRunPayload
            | Self::PromptTemplate { .. }
            | Self::MissingSkillMaterialization { .. }
            | Self::SessionEndedWithoutStopReason
            | Self::SessionBindingRejected
            | Self::BaselinePersist { .. }
            | Self::Repository(_)
            | Self::Session(_) => None,
        };
        NodeFailure::new(kind, self.to_string())
            .with_output(output)
            .with_source_chain(source_chain)
    }
}

/// Whether a node's stop should park an interactive node at `Pending` instead of reporting a
/// result. Completed and user-cancelled turns keep the conversation open for follow-up; a refusal
/// still fails the node.
fn pauses_interactive_node(interactive: bool, stop_reason: StopReason) -> bool {
    interactive
        && matches!(
            stop_reason,
            StopReason::EndTurn
                | StopReason::MaxTokens
                | StopReason::MaxTurnRequests
                | StopReason::Cancelled
        )
}

/// Runs the start → prompt → stop session chain for one agent node.
#[allow(clippy::too_many_arguments)]
async fn drive_agent_node(
    agent_runtime: &AgentRuntimeManager,
    pool: &RepositoryPool,
    agent_repository: &SqliteAgentDefinitionRepository,
    clock: &SystemClock,
    baselines_root: &Path,
    transitions: &WorkflowRunTransitions,
    node_run_id: &WorkflowNodeRunId,
    node: &WorkflowGraphNode,
    graph: &ora_application::WorkflowGraph,
    context: &ExecutionContext,
    scope_id: &ora_domain::WorkflowScopeId,
    variable_pool: &ora_application::WorkflowVariablePool,
) -> Result<AgentNodeOutcome, NodeExecutionError> {
    let config =
        node.agent_config
            .as_ref()
            .ok_or_else(|| NodeExecutionError::MissingAgentConfig {
                node_id: node.id.clone(),
            })?;
    let agent_ref = resolve_agent_ref(&config.executor.agent_cli)?;
    let run_payload = parse_workflow_run_payload(context.run.payload.as_deref())?;

    // Start the session without publishing it to the node yet. The owning prompt must win
    // admission before workflow UI loads can discover the session; failures in the preparation
    // block below stop this session so cancellation cannot leave an unbound actor behind.
    let started = agent_runtime
        .start_workflow_node_session(
            StartSessionRequest {
                workspace_id: context.run.workspace_id.to_string(),
                agent_ref,
                model: Some(config.executor.model_id.clone()),
            },
            crate::session_setup::SessionMcpSelection::Explicit(
                config
                    .mcps
                    .iter()
                    .filter(|mcp| mcp.enabled)
                    .map(|mcp| mcp.mcp_id.clone())
                    .collect(),
            ),
        )
        .await?;
    let session_id = SessionId::new(started.session.id);

    let outcome: Result<AgentNodeOutcome, NodeExecutionError> = async {
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());

        // Confirm the graph-declared model is both offered and active; no silent fallback.
        let (config_id, model_value) = match_model_value(
            &started.config_options,
            &config.executor.agent_cli,
            &config.executor.model_id,
        )?;
        let selected = started.config_options.iter().any(|option| {
            option.id.0.as_ref() == config_id
                && matches!(
                    &option.kind,
                    SessionConfigKind::Select(select) if select.current_value.0.as_ref() == model_value
                )
        });
        if !selected {
            return Err(NodeExecutionError::WorkflowModelNotFound {
                agent_ref: config.executor.agent_cli.clone(),
                model_id: config.executor.model_id.clone(),
            });
        }

        // Resolve the role's system instructions from the agents catalog; an empty role means no
        // system-instructions block is sent. Name is preferred; the id is a legacy fallback.
        let role_content = match &config.role_id {
            Some(role_id) if !role_id.trim().is_empty() => {
                let by_name = agent_repository
                    .find_agent_definition_by_name(&Namespace::local(), role_id)?;
                let definition = if by_name.is_some() {
                    by_name
                } else {
                    agent_repository.find_agent_definition(&AgentDefinitionId::new(role_id))?
                };
                definition.map(|definition| definition.content)
            }
            _ => None,
        };

        // Render {{#node.variable.path#}} placeholders in the node's prompt against the committed
        // variable pool, so an unresolvable reference fails the node before it starts.
        let mut rendered_node = node.clone();
        if let Some(agent_config) = rendered_node.agent_config.as_mut() {
            agent_config.prompt = render_variable_template(&config.prompt, variable_pool).map_err(
                |source| NodeExecutionError::PromptTemplate {
                    node_id: node.id.clone(),
                    source,
                },
            )?;
        }

        // Assemble one explicit workflow handoff while preserving leading slash-command parsing.
        let iteration = repository
            .list_node_runs(&context.run.id)?
            .iter()
            .find(|row| row.id == *node_run_id)
            .and_then(|row| row.iteration);
        let previous =
            repository.find_last_failed_attempt(&context.run.id, &node.id, iteration)?;
        let previous_failure =
            previous_failure_for_injection(run_payload.inject_last_failure, previous.as_ref());
        let node_runs = repository.list_node_runs_in_scope(scope_id)?;
        let workspace_root = agent_runtime.workspace_cwd(&context.workspace.id)?;
        let required_skills = resolve_required_skills(
            &run_payload,
            &node.id,
            &config.skills,
            &workspace_root,
        )?;
        let prompt = assemble_workflow_prompt(WorkflowPromptRequest {
            node: &rendered_node,
            graph: Some(graph),
            worktree_root: &workspace_root,
            role_content: role_content.as_deref(),
            graph_json: &context.graph_json,
            run_input: context.run.input.as_deref(),
            node_runs: &node_runs,
            required_skills: &required_skills,
            locale: run_payload.locale,
            previous_failure: previous_failure.as_ref(),
        });
        if let Some(previous) = previous_failure.as_ref() {
            repository.record_node_injected_failure(
                node_run_id,
                &render_previous_failure(previous, run_payload.locale),
            )?;
        }

        // Snapshot the worktree before this node runs so its completion diff is the node's own
        // incremental change (previous nodes' changes are already in the baseline).
        record_pre_node_checkpoint(
            &repository,
            &workspace_root,
            &context.run.id,
            &node.id,
            node_run_id,
            context.run.snapshot_id.as_ref(),
            clock.now_timestamp_millis(),
        )?;
        let baseline = capture_worktree_snapshot(&workspace_root);

        let mut stream = agent_runtime
            .prompt_session(PromptSessionRequest {
                session_id: session_id.to_string(),
                prompt,
                record_prompt: None,
                // The node's executor model was applied by the `startSession` above, which is
                // still holding this session's provider; there is no attach left to carry one.
                model: None,
            })
            .await?;

        // Publish the binding only after the actor accepts the owning prompt. A workflow chat
        // load can now only arrive while that prompt is active, where it joins as a follower
        // instead of unloading the new provider session and racing the prompt with session_busy.
        let now = clock.now_timestamp_millis();
        match repository.bind_node_run_session(node_run_id, &session_id, now)? {
            BindWorkflowNodeSessionResult::Bound => {
                agent_runtime.publish_workflow_node_session(&session_id)?;
            }
            BindWorkflowNodeSessionResult::NotRunning
            | BindWorkflowNodeSessionResult::NotFound => {
                return Err(NodeExecutionError::SessionBindingRejected);
            }
        }

        // Consume the owning prompt stream while `load_session` followers receive the same live
        // turn.
        let mut accumulator = AssistantOutputAccumulator::default();
        let mut stop_reason = None;
        while let Some(event) = stream.recv().await {
            match event? {
                PromptSessionEvent::SessionUpdate { update, .. } => {
                    accumulator.consume(&update);
                }
                PromptSessionEvent::PermissionRequest(_) => {}
                // The re-sent prompt answers the node afresh; text the stalled attempt got out
                // before Ora gave up on it is not part of the deliverable.
                PromptSessionEvent::Retrying { .. } => {
                    accumulator = AssistantOutputAccumulator::default();
                }
                PromptSessionEvent::Completed {
                    stop_reason: reason,
                    ..
                } => {
                    stop_reason = Some(reason);
                    break;
                }
            }
        }
        let stop_reason = stop_reason.ok_or(NodeExecutionError::SessionEndedWithoutStopReason)?;

        // An interactive node's first turn parks the node instead of completing it: the session
        // stays open for follow-up turns and the baseline is persisted for completion-time diffing.
        // A baseline is execution provenance, not a prerequisite, so a missing
        // (oversized/unavailable) baseline or a failed write still parks the node; its completion
        // later reports no file changes rather than failing the run.
        if pauses_interactive_node(config.interactive, stop_reason) {
            if let Some(baseline) = baseline.as_ref()
                && let Err(error) = persist_worktree_baseline(baselines_root, node_run_id, baseline)
            {
                ora_warn!(node_run_id = %node_run_id, error = %error, "failed to persist worktree baseline; the node still parks and its completion reports no file changes");
            }
            // The park commits through the shared transition sink so the awaiting state is
            // observable through the same invalidation channel as every engine transition
            // (ADR D7); a guard rejection (cancel or completion won the race) stays a no-op.
            transitions.transition_node_run_status(
                node_run_id,
                WorkflowNodeStatus::Running,
                WorkflowNodeStatus::Pending,
            )?;
            return Ok(AgentNodeOutcome::AwaitingInput);
        }

        // Stop the node's session; the Ora record stays queryable.
        agent_runtime
            .stop_session(StopSessionRequest {
                session_id: session_id.to_string(),
            })
            .await?;

        // Record the worktree delta since this node started: the baseline was captured before the
        // prompt, so only this node's own changes are reported, not earlier nodes' work.
        let file_changes = compute_file_changes(
            baseline.as_ref(),
            capture_worktree_snapshot(&workspace_root).as_ref(),
        );

        // Parse the optional structured value only after the final response is settled. The raw
        // response remains available to both successful completion and schema-failure reporting.
        let (output, structured_output) = apply_output_contract(
            config.output_contract.as_ref(),
            accumulator.into_output(),
            &node.id,
        )?;
        Ok(AgentNodeOutcome::Completed {
            output,
            structured_output,
            stop_reason,
            file_changes,
        })
    }
    .await;

    if outcome.is_err()
        && let Err(error) = agent_runtime
            .stop_session(StopSessionRequest {
                session_id: session_id.to_string(),
            })
            .await
    {
        // The original node error remains the actionable failure. Cleanup is best-effort, but the
        // warning keeps a leaked Running session diagnosable instead of silently masking it.
        ora_warn!(session_id = %session_id, error = %error, "failed to stop workflow session after node setup failed");
    }
    if outcome.is_err()
        && let Err(error) = agent_runtime
            .discard_unpublished_workflow_node_session(&session_id)
            .await
    {
        // A failed pre-binding Session must remain hidden in memory if cleanup cannot remove its
        // row. Keeping the original node error preserves the actionable failure for the engine.
        ora_warn!(session_id = %session_id, error = %error, "failed to discard unpublished workflow session after node setup failed");
    }
    outcome
}

/// Resolves a completed node's final response into the output variables to commit.
///
/// A structured contract parses the response as a JSON object and validates it against the schema.
/// The raw result remains the node's stable `output`; absent and legacy text contracts perform no
/// additional parsing.
pub(crate) fn apply_output_contract(
    contract: Option<&AgentOutputContract>,
    final_text: Option<String>,
    node_id: &str,
) -> Result<(Option<String>, Option<serde_json::Value>), NodeExecutionError> {
    match contract {
        None => Ok((final_text, None)),
        Some(AgentOutputContract::None) => Ok((final_text, None)),
        Some(AgentOutputContract::Text) => Ok((final_text, None)),
        Some(AgentOutputContract::Structured {
            schema,
            text_exposure: _,
        }) => {
            let Some(text) = final_text else {
                return Err(NodeExecutionError::StructuredOutput {
                    node_id: node_id.to_string(),
                    source: StructuredOutputError::NotJsonObject {
                        reason: "the agent produced no final response".to_string(),
                    },
                    output: None,
                });
            };
            let structured = match extract_json_object(&text).and_then(|value| {
                validate_against_schema(&value, schema)?;
                Ok(value)
            }) {
                Ok(structured) => structured,
                Err(source) => {
                    return Err(NodeExecutionError::StructuredOutput {
                        node_id: node_id.to_string(),
                        source,
                        output: Some(text),
                    });
                }
            };
            Ok((Some(text), Some(structured)))
        }
    }
}

/// Reads the agent identity a graph node declares.
///
/// Which agents exist depends on installed plugins, so the graph's value is carried through as
/// written instead of being checked against a fixed set. Whether that agent is installed is
/// answered later by the runtime, which reports an unavailable provider rather than an
/// unsupported one.
pub(super) fn resolve_agent_ref(value: &str) -> Result<ContractAgentRef, NodeExecutionError> {
    if value.trim().is_empty() {
        return Err(NodeExecutionError::MissingAgentRef);
    }
    Ok(value.to_string())
}

/// Finds the model option and the value to select for the graph's `modelId`.
///
/// Matching follows the confirmed order: a `Model`-category select, falling back to the sole
/// select option; then an exact value match, then a label-contains match. No match fails the
/// node instead of silently using the CLI default.
fn match_model_value(
    config_options: &[SessionConfigOption],
    agent_ref: &str,
    model_id: &str,
) -> Result<(String, String), NodeExecutionError> {
    let model_option = config_options
        .iter()
        .find(|option| matches!(option.category, Some(SessionConfigOptionCategory::Model)))
        .or_else(|| {
            let selects: Vec<&SessionConfigOption> = config_options
                .iter()
                .filter(|option| matches!(option.kind, SessionConfigKind::Select(_)))
                .collect();
            (selects.len() == 1).then_some(selects[0])
        })
        .ok_or_else(|| NodeExecutionError::WorkflowModelNotFound {
            agent_ref: agent_ref.to_string(),
            model_id: model_id.to_string(),
        })?;
    let SessionConfigKind::Select(select) = &model_option.kind else {
        return Err(NodeExecutionError::WorkflowModelNotFound {
            agent_ref: agent_ref.to_string(),
            model_id: model_id.to_string(),
        });
    };
    let options: Vec<&SessionConfigSelectOption> = match &select.options {
        SessionConfigSelectOptions::Ungrouped(options) => options.iter().collect(),
        SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| group.options.iter())
            .collect(),
        // New option container shapes require an explicit selection policy before
        // workflow execution can choose a model from them.
        _ => {
            return Err(NodeExecutionError::WorkflowModelNotFound {
                agent_ref: agent_ref.to_string(),
                model_id: model_id.to_string(),
            });
        }
    };
    let matched = options
        .iter()
        .find(|option| option.value.0.as_ref() == model_id || option.name.contains(model_id));
    match matched {
        Some(option) => Ok((model_option.id.0.to_string(), option.value.0.to_string())),
        None => Err(NodeExecutionError::WorkflowModelNotFound {
            agent_ref: agent_ref.to_string(),
            model_id: model_id.to_string(),
        }),
    }
}

/// Resolves the current node's enabled skills exclusively from the receipt frozen at deployment.
fn resolve_required_skills(
    payload: &WorkflowRunPayload,
    node_id: &str,
    skills: &[AgentSkill],
    worktree_root: &Path,
) -> Result<Vec<RequiredWorkflowSkill>, NodeExecutionError> {
    let bindings = payload.skill_materialization.bindings_for_node(node_id);
    let mut required = Vec::new();
    let mut seen_skill_ids = HashSet::new();
    for skill in skills.iter().filter(|skill| skill.enabled) {
        if !seen_skill_ids.insert(skill.skill_id.clone()) {
            continue;
        }
        let binding = bindings
            .iter()
            .find(|binding| binding.skill_id == skill.skill_id)
            .ok_or_else(|| NodeExecutionError::MissingSkillMaterialization {
                node_id: node_id.to_string(),
                skill_id: skill.skill_id.clone(),
            })?;
        required.push(RequiredWorkflowSkill {
            invocation_name: binding.invocation_name.clone(),
            package_paths: binding.absolute_package_paths(worktree_root),
        });
    }
    Ok(required)
}

/// Renders an ACP stop reason as its snake-case label, matching the wire form persisted on the
/// node's completion payload.
pub(crate) fn stop_reason_label(reason: StopReason) -> String {
    match reason {
        StopReason::EndTurn => "end_turn".to_string(),
        StopReason::MaxTokens => "max_tokens".to_string(),
        StopReason::MaxTurnRequests => "max_turn_requests".to_string(),
        StopReason::Refusal => "refusal".to_string(),
        StopReason::Cancelled => "cancelled".to_string(),
        // A newer ACP stop reason has no label Ora can persist faithfully; the caller decides
        // whether to fall back to a failure.
        _ => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol_schema::v1::{
        SessionConfigId, SessionConfigSelect, SessionConfigValueId,
    };
    use ora_application::StructuredTextExposure;
    use ora_contracts::WorkflowRunLocale;
    use ora_utils::path::StrictRelativePath;
    use pretty_assertions::assert_eq;

    fn select_option(value: &str, name: &str) -> SessionConfigSelectOption {
        SessionConfigSelectOption::new(
            SessionConfigValueId::new(value.to_string()),
            name.to_string(),
        )
    }

    /// Verifies completed and user-cancelled turns park an interactive node.
    #[test]
    fn pauses_interactive_node_parks_completed_and_cancelled_turns() {
        assert!(pauses_interactive_node(true, StopReason::EndTurn));
        assert!(pauses_interactive_node(true, StopReason::MaxTokens));
        assert!(pauses_interactive_node(true, StopReason::MaxTurnRequests));
        // A refusal still fails the node, while an explicit prompt cancellation yields control.
        assert!(!pauses_interactive_node(true, StopReason::Refusal));
        assert!(pauses_interactive_node(true, StopReason::Cancelled));
        // Non-interactive nodes keep the existing complete-on-EndTurn behavior.
        assert!(!pauses_interactive_node(false, StopReason::EndTurn));
    }

    fn model_option(options: Vec<SessionConfigSelectOption>) -> SessionConfigOption {
        SessionConfigOption::new(
            SessionConfigId::new("model".to_string()),
            "Model",
            SessionConfigKind::Select(SessionConfigSelect::new(
                SessionConfigValueId::new("current".to_string()),
                SessionConfigSelectOptions::Ungrouped(options),
            )),
        )
        .category(SessionConfigOptionCategory::Model)
    }

    /// Verifies a graph's agent identity is carried through instead of checked against a set.
    ///
    /// Which agents exist depends on installed plugins, so an identity this build has never heard
    /// of must reach the runtime and be reported as unavailable there, not rejected here.
    #[test]
    fn resolve_agent_ref_accepts_any_named_agent() {
        assert_eq!(
            ["ora-space.codex", "acme.my-agent"].map(|value| resolve_agent_ref(value).unwrap()),
            ["ora-space.codex".to_string(), "acme.my-agent".to_string()]
        );
        assert!(matches!(
            resolve_agent_ref("   "),
            Err(NodeExecutionError::MissingAgentRef)
        ));
    }

    #[test]
    fn match_model_value_prefers_exact_value_then_label() {
        let options = vec![
            select_option("fast", "Fast model"),
            select_option("deepseek/deepseek-v4-pro", "DeepSeek V4 Pro"),
        ];
        let config = vec![model_option(options)];
        assert_eq!(
            match_model_value(&config, "open_code", "deepseek/deepseek-v4-pro").unwrap(),
            ("model".to_string(), "deepseek/deepseek-v4-pro".to_string())
        );
        // A label-contains match also works (case-sensitive, per the confirmed model rule).
        assert_eq!(
            match_model_value(&config, "open_code", "DeepSeek").unwrap(),
            ("model".to_string(), "deepseek/deepseek-v4-pro".to_string())
        );
    }

    #[test]
    fn match_model_value_fails_when_no_option_matches() {
        let config = vec![model_option(vec![select_option("fast", "Fast model")])];
        assert!(matches!(
            match_model_value(&config, "open_code", "missing-model"),
            Err(NodeExecutionError::WorkflowModelNotFound { .. })
        ));
    }

    #[test]
    fn match_model_value_falls_back_to_the_lone_select() {
        let option = SessionConfigOption::new(
            SessionConfigId::new("model".to_string()),
            "Model",
            SessionConfigKind::Select(SessionConfigSelect::new(
                SessionConfigValueId::new("smart".to_string()),
                SessionConfigSelectOptions::Ungrouped(vec![select_option("smart", "Smart")]),
            )),
        );
        assert_eq!(
            match_model_value(&[option], "open_code", "smart").unwrap(),
            ("model".to_string(), "smart".to_string())
        );
    }

    /// The executor accepts only the typed locale and receipt persisted by run deployment.
    #[test]
    fn workflow_run_payload_reads_the_execution_metadata_frozen_on_the_run() {
        let expected = WorkflowRunPayload::new(WorkflowRunLocale::EnUs, Default::default());
        let payload = serde_json::to_string(&expected).unwrap();
        assert_eq!(
            parse_workflow_run_payload(Some(&payload)).unwrap(),
            expected
        );
        assert!(matches!(
            parse_workflow_run_payload(None),
            Err(NodeExecutionError::InvalidRunPayload)
        ));
    }

    /// A structured contract with `includeFinalText` returns the raw text and the parsed object.
    #[test]
    fn apply_output_contract_parses_and_keeps_text_when_requested() {
        let contract = AgentOutputContract::Structured {
            schema: serde_json::json!({
                "type": "object",
                "properties": { "approved": { "type": "boolean" } },
                "required": ["approved"]
            }),
            text_exposure: StructuredTextExposure::IncludeFinalText,
        };
        let (output, structured) = apply_output_contract(
            Some(&contract),
            Some(r#"{"approved": true}"#.to_string()),
            "review",
        )
        .unwrap();
        assert_eq!(output, Some(r#"{"approved": true}"#.to_string()));
        assert_eq!(structured, Some(serde_json::json!({ "approved": true })));
    }

    /// A structured-only contract still keeps the node's stable raw `output`.
    #[test]
    fn apply_output_contract_keeps_raw_output_when_structured_only() {
        let contract = AgentOutputContract::Structured {
            schema: serde_json::json!({ "type": "object" }),
            text_exposure: StructuredTextExposure::StructuredOnly,
        };
        let (output, structured) = apply_output_contract(
            Some(&contract),
            Some(r#"{"approved": true}"#.to_string()),
            "review",
        )
        .unwrap();
        assert_eq!(output, Some(r#"{"approved": true}"#.to_string()));
        assert_eq!(structured, Some(serde_json::json!({ "approved": true })));
    }

    /// A structured response that is not a JSON object or violates the schema fails the node.
    #[test]
    fn apply_output_contract_fails_on_invalid_or_nonconforming_json() {
        let contract = AgentOutputContract::Structured {
            schema: serde_json::json!({
                "type": "object",
                "properties": { "approved": { "type": "boolean" } },
                "required": ["approved"]
            }),
            text_exposure: StructuredTextExposure::StructuredOnly,
        };
        let invalid_json =
            apply_output_contract(Some(&contract), Some("no json here".to_string()), "review")
                .unwrap_err();
        let report = invalid_json.into_failure_report();
        assert!(report.message.contains("structured output failed"));
        assert_eq!(report.output, Some("no json here".to_string()));
        assert_eq!(report.kind, NodeFailureKind::StructuredOutput);
        assert!(!report.source_chain.is_empty());
        assert!(matches!(
            apply_output_contract(
                Some(&contract),
                Some(r#"{"approved": "yes"}"#.to_string()),
                "review"
            ),
            Err(NodeExecutionError::StructuredOutput { .. })
        ));
    }

    /// A session that ends without a stop reason has no underlying source error to chain.
    #[test]
    fn into_failure_report_for_session_ended_without_stop_reason_has_empty_chain() {
        let report = NodeExecutionError::SessionEndedWithoutStopReason.into_failure_report();
        assert_eq!(report.kind, NodeFailureKind::SessionEndedWithoutStopReason);
        assert_eq!(report.source_chain, Vec::<String>::new());
        assert_eq!(report.output, None);
    }

    /// A missing structured contract still preserves the node's raw text output.
    #[test]
    fn apply_output_contract_keeps_text_when_absent() {
        assert_eq!(
            apply_output_contract(None, Some("text".into()), "a").unwrap(),
            (Some("text".to_string()), None)
        );
    }

    /// Node execution uses the frozen receipt for invocation and placement, never the live catalog.
    #[test]
    fn required_skills_resolve_only_from_the_frozen_materialization_receipt() {
        let payload = WorkflowRunPayload::new(
            WorkflowRunLocale::ZhCn,
            ora_application::SkillMaterializationReceipt {
                bindings: vec![ora_application::MaterializedSkillBinding {
                    node_id: "review-node".to_string(),
                    skill_id: "catalog-id".to_string(),
                    invocation_name: "review".to_string(),
                    package_paths: vec![
                        StrictRelativePath::parse(".plugin/skills/review").unwrap(),
                    ],
                }],
            },
        );
        let skills = vec![AgentSkill {
            skill_id: "catalog-id".to_string(),
            enabled: true,
        }];
        let worktree_root = Path::new("worktrees").join("run-1");

        assert_eq!(
            resolve_required_skills(&payload, "review-node", &skills, &worktree_root).unwrap(),
            vec![RequiredWorkflowSkill {
                invocation_name: "review".to_string(),
                package_paths: vec![worktree_root.join(".plugin").join("skills").join("review")],
            }]
        );
        assert!(matches!(
            resolve_required_skills(&payload, "other-node", &skills, &worktree_root),
            Err(NodeExecutionError::MissingSkillMaterialization { node_id, skill_id })
                if node_id == "other-node" && skill_id == "catalog-id"
        ));
    }
}
