use super::prompt::{RequiredWorkflowSkill, WorkflowPromptRequest, assemble_workflow_prompt};
use crate::agent_runtime::{AgentRuntimeManager, WarmOwner};
use crate::clock::SystemClock;
use crate::error::BackendError;
use agent_client_protocol_schema::v1::ContentBlock;
use agent_client_protocol_schema::v1::MessageId;
use agent_client_protocol_schema::v1::SessionUpdate;
use agent_client_protocol_schema::v1::StopReason;
use agent_client_protocol_schema::v1::ToolCallId;
use agent_client_protocol_schema::v1::{
    SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
    SessionConfigSelectOptions,
};
use ora_application::{
    AgentDefinitionRepository, AgentSkill, BindWorkflowNodeSessionResult, Clock, ExecutionContext,
    FileChange, NodeExecutor, RepositoryError, WorkflowGraphNode, WorkflowRunCallback,
    WorkflowRunEngineRepository, WorkflowRunPayload,
};
use ora_contracts::{
    AgentRef as ContractAgentRef, AttachSessionRequest, PromptSessionEvent, PromptSessionRequest,
    SetSessionConfigRequest, StopSessionRequest, WarmSessionRequest, WarmSessionTarget,
};
use ora_db::{RepositoryPool, SqliteAgentDefinitionRepository, SqliteWorkflowRunEngineRepository};
use ora_domain::{
    AgentDefinitionId, Namespace, SessionId, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId,
};
use ora_logging::ora_warn;
use similar::{ChangeTag, TextDiff};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use thiserror::Error;

/// Executes one agent node through a real Ora session, reporting completion to the run engine.
///
/// `dispatch` spawns a background task that warms, attaches, configures, prompts, and stops one
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
}

impl WorkflowRunNodeExecutor {
    /// Builds an executor from the session runtime, persistence, role catalog, and engine callback.
    pub fn new(
        agent_runtime: Arc<AgentRuntimeManager>,
        pool: RepositoryPool,
        agent_repository: SqliteAgentDefinitionRepository,
        callback: Arc<dyn WorkflowRunCallback>,
        clock: SystemClock,
        baselines_root: PathBuf,
    ) -> Self {
        Self {
            agent_runtime,
            pool,
            agent_repository,
            callback,
            clock,
            baselines_root,
        }
    }
}

impl NodeExecutor for WorkflowRunNodeExecutor {
    fn dispatch(
        &self,
        node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        context: &ExecutionContext,
    ) {
        let agent_runtime = self.agent_runtime.clone();
        let pool = self.pool.clone();
        let agent_repository = self.agent_repository.clone();
        let callback = self.callback.clone();
        let clock = self.clock;
        let baselines_root = self.baselines_root.clone();
        let node_run_id = node_run_id.clone();
        let node = node.clone();
        let context = context.clone();
        tokio::spawn(async move {
            match drive_agent_node(
                &agent_runtime,
                &pool,
                &agent_repository,
                &clock,
                &baselines_root,
                &node_run_id,
                &node,
                &context,
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
                    let callback = callback.clone();
                    let run_id = context.run.id.clone();
                    let node_run_id = node_run_id.clone();
                    let message = error.message();
                    let join = tokio::task::spawn_blocking(move || {
                        callback.fail_node(&run_id, &node_run_id, message, None);
                    });
                    if let Err(source) = join.await {
                        ora_warn!("workflow node failure callback panicked: {source}");
                    }
                }
            }
        });
    }
}

/// The result of one driven agent node turn.
enum AgentNodeOutcome {
    /// The node finished and reports completion or failure through the callback.
    Completed {
        output: Option<String>,
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
    /// Renders the actionable message surfaced to the failed node.
    fn message(&self) -> String {
        self.to_string()
    }
}

/// Total bytes of tracked-file content a worktree baseline may hold before capture aborts.
///
/// A baseline is execution provenance, not a prerequisite for running a node: exceeding this cap
/// aborts the snapshot and the node continues with an empty diff, rather than copying an unbounded
/// amount of repository content to disk.
const MAX_BASELINE_BYTES: u64 = 64 * 1024 * 1024;

/// Captures every worktree-visible file (tracked and untracked, gitignore-respecting) as
/// worktree-relative path → content, or `None` when git is unavailable or the content exceeds
/// [`MAX_BASELINE_BYTES`].
///
/// Unlike `git status --porcelain`, which folds untracked directories into a single `?? dir/`
/// entry and omits clean tracked files, `git ls-files -co` expands both: a node that creates
/// files inside a new directory (e.g. `openspec/...`) or edits an already-committed file for the
/// first time still shows up in the before/after delta. `None` deliberately means "no baseline is
/// available", never "the tree was empty", so callers report no diff instead of treating the whole
/// tree as new.
pub(crate) fn capture_worktree_snapshot(
    worktree_root: &Path,
) -> Option<BTreeMap<String, Option<String>>> {
    let Ok(output) = Command::new("git")
        .args(["ls-files", "-co", "--exclude-standard", "-z"])
        .current_dir(worktree_root)
        .output()
    else {
        return None;
    };
    let mut snapshot = BTreeMap::new();
    let mut total_bytes: u64 = 0;
    for path in String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|entry| !entry.is_empty())
    {
        // Check the file's size before reading it, so a single oversized file cannot be read whole
        // into memory before the cap trips. A deleted tracked file has no metadata and reads as
        // `None`; a directory entry can only be an in-index submodule, which we skip as content.
        let Ok(metadata) = std::fs::metadata(worktree_root.join(path)) else {
            snapshot.insert(path.to_string(), None);
            continue;
        };
        total_bytes = total_bytes.saturating_add(metadata.len());
        if total_bytes > MAX_BASELINE_BYTES {
            return None;
        }
        let content = std::fs::read_to_string(worktree_root.join(path)).ok();
        snapshot.insert(path.to_string(), content);
    }
    Some(snapshot)
}

/// Diffs the worktree state captured before a node ran against the state after it finished, so
/// only this node's incremental changes are reported. A missing baseline or current snapshot
/// yields an empty diff — it is never treated as an empty worktree that makes every file new.
pub(crate) fn compute_file_changes(
    baseline: Option<&BTreeMap<String, Option<String>>>,
    current: Option<&BTreeMap<String, Option<String>>>,
) -> Vec<FileChange> {
    let (Some(baseline), Some(current)) = (baseline, current) else {
        return Vec::new();
    };
    let paths: BTreeSet<&String> = baseline.keys().chain(current.keys()).collect();
    let mut changes = Vec::new();
    for path in paths {
        let before = baseline.get(path).and_then(Clone::clone);
        let after = current.get(path).and_then(Clone::clone);
        let (additions, deletions) = match (before, after) {
            (None, Some(after)) => (count_lines(&after), 0),
            (Some(before), None) => (0, count_lines(&before)),
            (Some(before), Some(after)) => line_diff_counts(&before, &after),
            (None, None) => continue,
        };
        if additions > 0 || deletions > 0 {
            changes.push(FileChange {
                path: path.clone(),
                additions,
                deletions,
            });
        }
    }
    changes
}

/// Counts the added and removed lines between two file contents.
fn line_diff_counts(before: &str, after: &str) -> (u64, u64) {
    let diff = TextDiff::from_lines(before, after);
    let additions = diff
        .iter_all_changes()
        .filter(|change| change.tag() == ChangeTag::Insert)
        .count() as u64;
    let deletions = diff
        .iter_all_changes()
        .filter(|change| change.tag() == ChangeTag::Delete)
        .count() as u64;
    (additions, deletions)
}

/// Counts the lines of a file for new-file additions or whole-file deletions.
fn count_lines(content: &str) -> u64 {
    content.lines().count() as u64
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

/// Persists the worktree snapshot captured when an interactive node started, so the completion
/// flow can later diff only this node's changes. Lives under a dedicated baselines root, never
/// the database or the worktree itself.
fn persist_worktree_baseline(
    baselines_root: &Path,
    node_run_id: &WorkflowNodeRunId,
    baseline: &BTreeMap<String, Option<String>>,
) -> Result<(), NodeExecutionError> {
    std::fs::create_dir_all(baselines_root).map_err(|source| {
        NodeExecutionError::BaselinePersist {
            node_id: node_run_id.to_string(),
            source,
        }
    })?;
    let path = baselines_root.join(format!("{}.json", node_run_id.as_ref()));
    let json =
        serde_json::to_vec(baseline).map_err(|error| NodeExecutionError::BaselinePersist {
            node_id: node_run_id.to_string(),
            source: error.into(),
        })?;
    std::fs::write(path, json).map_err(|source| NodeExecutionError::BaselinePersist {
        node_id: node_run_id.to_string(),
        source,
    })?;
    Ok(())
}

/// Runs the warm → attach → model → prompt → stop session chain for one agent node.
#[allow(clippy::too_many_arguments)]
async fn drive_agent_node(
    agent_runtime: &AgentRuntimeManager,
    pool: &RepositoryPool,
    agent_repository: &SqliteAgentDefinitionRepository,
    clock: &SystemClock,
    baselines_root: &Path,
    node_run_id: &WorkflowNodeRunId,
    node: &WorkflowGraphNode,
    context: &ExecutionContext,
) -> Result<AgentNodeOutcome, NodeExecutionError> {
    let config =
        node.agent_config
            .as_ref()
            .ok_or_else(|| NodeExecutionError::MissingAgentConfig {
                node_id: node.id.clone(),
            })?;
    let agent_ref = resolve_agent_ref(&config.executor.agent_cli)?;
    let run_payload = parse_workflow_run_payload(context.run.payload.as_deref())?;

    // Warm a reusable provider session for this run's workspace.
    let warm = agent_runtime
        .warm_session_for_owner(
            WarmSessionRequest {
                target: WarmSessionTarget::Workspace {
                    workspace_id: context.run.workspace_id.to_string(),
                },
                agent_ref,
            },
            WarmOwner::WorkflowNode {
                run_id: context.run.id.to_string(),
                node_id: node.id.clone(),
            },
        )
        .await?;

    // Attach the warm session without publishing it to the node yet. The owning prompt must win
    // admission before workflow UI loads can discover the session; failures in the preparation
    // block below stop this attached session so cancellation cannot leave an unbound actor behind.
    let attach = agent_runtime
        .attach_workflow_node_session(AttachSessionRequest {
            session_id: warm.session_id.clone(),
            workspace_id: context.run.workspace_id.to_string(),
        })
        .await?;
    let session_id = SessionId::new(attach.session.id);

    let outcome: Result<AgentNodeOutcome, NodeExecutionError> = async {
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());

        // Select the graph-declared model from the warm-advertised options; no silent fallback.
        let (config_id, model_value) = match_model_value(
            &warm.config_options,
            &config.executor.agent_cli,
            &config.executor.model_id,
        )?;
        agent_runtime
            .set_session_config(SetSessionConfigRequest {
                session_id: warm.session_id.clone(),
                config_id,
                value: model_value,
            })
            .await?;

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

        // Assemble one explicit workflow handoff while preserving leading slash-command parsing.
        let node_runs = repository.list_node_runs(&context.run.id)?;
        let workspace_root = agent_runtime.workspace_cwd(&context.workspace.id)?;
        let required_skills = resolve_required_skills(
            &run_payload,
            &node.id,
            &config.skills,
            &workspace_root,
        )?;
        let prompt = assemble_workflow_prompt(WorkflowPromptRequest {
            node,
            worktree_root: &workspace_root,
            role_content: role_content.as_deref(),
            graph_json: &context.graph_json,
            run_input: context.run.input.as_deref(),
            node_runs: &node_runs,
            required_skills: &required_skills,
            locale: run_payload.locale,
        });

        // Snapshot the worktree before this node runs so its completion diff is the node's own
        // incremental change (previous nodes' changes are already in the baseline).
        let baseline = capture_worktree_snapshot(&workspace_root);

        let mut stream = agent_runtime
            .prompt_session(PromptSessionRequest {
                session_id: warm.session_id.clone(),
                prompt,
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
                PromptSessionEvent::SessionUpdate { update } => {
                    accumulator.consume(&update);
                }
                PromptSessionEvent::PermissionRequest(_) => {}
                PromptSessionEvent::Completed {
                    stop_reason: reason,
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
            let now = clock.now_timestamp_millis();
            repository.transition_node_run_status(
                node_run_id,
                WorkflowNodeStatus::Running,
                WorkflowNodeStatus::Pending,
                now,
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

        Ok(AgentNodeOutcome::Completed {
            // Apply the node's output policy at the single place the completed output is produced,
            // so both `complete_node` and the refusal/unknown failure branches reuse the same
            // value: a `None` policy withholds the assistant deliverable on success and failure.
            output: config.output_policy.apply(accumulator.into_output()),
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

/// Parses the immutable locale and skill placement receipt captured when the run was created.
fn parse_workflow_run_payload(
    payload: Option<&str>,
) -> Result<WorkflowRunPayload, NodeExecutionError> {
    payload
        .and_then(|payload| serde_json::from_str(payload).ok())
        .ok_or(NodeExecutionError::InvalidRunPayload)
}

/// Reports one finished turn to the engine according to the confirmed stop-reason mapping.
fn report_outcome(
    callback: &Arc<dyn WorkflowRunCallback>,
    run_id: &WorkflowRunId,
    node_run_id: &WorkflowNodeRunId,
    outcome: AgentNodeOutcome,
) {
    let AgentNodeOutcome::Completed {
        output,
        stop_reason,
        file_changes,
    } = outcome
    else {
        // An interactive node parked at `Pending` reports nothing; the human drives completion.
        return;
    };
    match stop_reason {
        StopReason::EndTurn => callback.complete_node(
            run_id,
            node_run_id,
            output,
            Some("end_turn".to_string()),
            file_changes,
        ),
        StopReason::MaxTokens => callback.complete_node(
            run_id,
            node_run_id,
            output,
            Some("max_tokens".to_string()),
            file_changes,
        ),
        StopReason::MaxTurnRequests => callback.complete_node(
            run_id,
            node_run_id,
            output,
            Some("max_turn_requests".to_string()),
            file_changes,
        ),
        StopReason::Refusal => callback.fail_node(
            run_id,
            node_run_id,
            "agent refused the request".to_string(),
            output,
        ),
        StopReason::Cancelled => {
            // Non-interactive cancellation belongs to the run cancel flow; interactive turns
            // already returned through the awaiting-input branch above.
        }
        // A newer ACP stop reason has semantics this executor cannot safely map to a
        // successful workflow transition.
        _ => callback.fail_node(
            run_id,
            node_run_id,
            "agent stopped for a reason this Ora version does not recognize".to_string(),
            output,
        ),
    }
}

/// Reads the agent identity a graph node declares.
///
/// Which agents exist depends on installed plugins, so the graph's value is carried through as
/// written instead of being checked against a fixed set. Whether that agent is installed is
/// answered later by the runtime, which reports an unavailable provider rather than an
/// unsupported one.
fn resolve_agent_ref(value: &str) -> Result<ContractAgentRef, NodeExecutionError> {
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

/// Accumulates only the final assistant deliverable produced by the node's prompt turn.
///
/// A turn can contain explanation text, tool calls, then a final answer; the node's output is the
/// final assistant message, not the concatenation of every text run. A changed `message_id` starts
/// a fresh message, and so does a position-claiming item (a new tool, the first plan, or non-text
/// content) interrupting a run that carries no `message_id`, mirroring the assembler's contiguity
/// rules so the automatic path matches the interactive path.
#[derive(Debug, Default)]
struct AssistantOutputAccumulator {
    message_id: Option<MessageId>,
    text: String,
    /// A position-claiming item interrupted the current implicit, no-`messageId` text run; the next
    /// no-id text starts a fresh final message.
    interrupted: bool,
    /// Tool ids already seen this turn, so an update to a known tool claims no new position.
    seen_tool_ids: HashSet<ToolCallId>,
    /// Whether a plan has already been seen, so a plan replacement claims no new position.
    has_plan: bool,
}

impl AssistantOutputAccumulator {
    /// Records assistant text while leaving the session history responsible for full conversation.
    fn consume(&mut self, update: &SessionUpdate) {
        match update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                let Some(text) = chunk_text(&chunk.content) else {
                    // Non-text content claims its own position, so it interrupts an implicit run.
                    self.interrupted = true;
                    return;
                };
                let id_changed = self.message_id.as_ref() != chunk.message_id.as_ref();
                let implicit_interrupted = chunk.message_id.is_none() && self.interrupted;
                if id_changed || implicit_interrupted {
                    self.message_id = chunk.message_id.clone();
                    self.text.clear();
                }
                self.interrupted = false;
                self.text.push_str(text);
            }
            // A tool opening, or the first update of an unknown tool, claims a new position and
            // interrupts an implicit run; an update to a known tool does not.
            SessionUpdate::ToolCall(call) => {
                self.interrupted |= self.seen_tool_ids.insert(call.tool_call_id.clone());
            }
            SessionUpdate::ToolCallUpdate(update) => {
                self.interrupted |= self.seen_tool_ids.insert(update.tool_call_id.clone());
            }
            // Only the first plan claims a position; a replacement does not interrupt.
            SessionUpdate::Plan(_) => {
                self.interrupted |= !self.has_plan;
                self.has_plan = true;
            }
            _ => {}
        }
    }

    /// Returns the assistant's scalar output, or `None` when no assistant text was produced.
    fn into_output(self) -> Option<String> {
        if self.text.is_empty() {
            return None;
        }
        Some(self.text)
    }
}

/// Extracts the text payload from a content block, ignoring non-text blocks.
fn chunk_text(block: &ContentBlock) -> Option<&str> {
    match block {
        ContentBlock::Text(text) => Some(&text.text),
        _ => None,
    }
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
        ContentChunk, MessageId, SessionConfigId, SessionConfigSelect, SessionConfigValueId,
        TextContent, ToolCall, ToolCallId, ToolCallUpdate, ToolCallUpdateFields,
    };
    use ora_contracts::WorkflowRunLocale;
    use ora_utils::path::StrictRelativePath;
    use pretty_assertions::assert_eq;

    fn select_option(value: &str, name: &str) -> SessionConfigSelectOption {
        SessionConfigSelectOption::new(
            SessionConfigValueId::new(value.to_string()),
            name.to_string(),
        )
    }

    fn agent_text_update(message_id: Option<&str>, text: &str) -> SessionUpdate {
        let mut chunk = ContentChunk::new(ContentBlock::Text(TextContent::new(text.to_string())));
        chunk.message_id = message_id.map(MessageId::new);
        SessionUpdate::AgentMessageChunk(chunk)
    }

    /// A missing baseline or current snapshot yields an empty diff, never "the whole tree is new".
    #[test]
    fn compute_file_changes_reports_empty_diff_when_a_side_is_missing() {
        let mut baseline = BTreeMap::new();
        baseline.insert("src/a.ts".to_string(), Some("one\n".to_string()));
        assert_eq!(compute_file_changes(None, Some(&baseline)), Vec::new());
        assert_eq!(compute_file_changes(Some(&baseline), None), Vec::new());
    }

    /// A turn that emits explanation then a final answer keeps only the final message, matching the
    /// interactive completion path that reads the last settled assistant message.
    #[test]
    fn assistant_output_accumulator_keeps_only_the_final_message() {
        let mut accumulator = AssistantOutputAccumulator::default();
        accumulator.consume(&agent_text_update(Some("msg-1"), "let me think "));
        accumulator.consume(&agent_text_update(Some("msg-2"), "final answer"));
        assert_eq!(accumulator.into_output(), Some("final answer".to_string()));
    }

    /// A no-`messageId` text run interrupted by a tool call starts a fresh final message, matching
    /// the assembler's contiguity rule.
    #[test]
    fn assistant_output_accumulator_keeps_only_the_final_implicit_message() {
        let mut accumulator = AssistantOutputAccumulator::default();
        accumulator.consume(&agent_text_update(None, "let me think "));
        accumulator.consume(&SessionUpdate::ToolCall(ToolCall::new(
            ToolCallId::new("t1"),
            "look up",
        )));
        accumulator.consume(&agent_text_update(None, "final answer"));
        assert_eq!(accumulator.into_output(), Some("final answer".to_string()));
    }

    /// An update to an already-known tool claims no new position, so a no-`messageId` text run on
    /// either side of it stays one message, matching the assembler.
    #[test]
    fn assistant_output_accumulator_does_not_break_on_a_known_tool_update() {
        let mut accumulator = AssistantOutputAccumulator::default();
        accumulator.consume(&agent_text_update(None, "foo "));
        accumulator.consume(&SessionUpdate::ToolCall(ToolCall::new(
            ToolCallId::new("t1"),
            "read",
        )));
        accumulator.consume(&agent_text_update(None, "bar "));
        accumulator.consume(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            ToolCallId::new("t1"),
            ToolCallUpdateFields::new(),
        )));
        accumulator.consume(&agent_text_update(None, "baz"));
        assert_eq!(accumulator.into_output(), Some("bar baz".to_string()));
    }

    #[test]
    fn compute_file_changes_reports_only_the_incremental_delta() {
        let mut baseline = BTreeMap::new();
        baseline.insert("src/a.ts".to_string(), Some("one\ntwo\n".to_string()));
        baseline.insert("src/b.ts".to_string(), Some("keep\n".to_string()));

        let mut current = BTreeMap::new();
        current.insert(
            "src/a.ts".to_string(),
            Some("one\ntwo\nthree\n".to_string()),
        );
        current.insert("src/b.ts".to_string(), None);
        current.insert("src/new.ts".to_string(), Some("fresh\n".to_string()));

        // a.ts gained a line, b.ts was deleted, new.ts was added; keep unchanged is excluded.
        assert_eq!(
            compute_file_changes(Some(&baseline), Some(&current)),
            vec![
                FileChange {
                    path: "src/a.ts".to_string(),
                    additions: 1,
                    deletions: 0
                },
                FileChange {
                    path: "src/b.ts".to_string(),
                    additions: 0,
                    deletions: 1
                },
                FileChange {
                    path: "src/new.ts".to_string(),
                    additions: 1,
                    deletions: 0
                },
            ]
        );
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

    /// Verifies a persisted worktree baseline round-trips through its side file.
    #[test]
    fn persist_worktree_baseline_round_trips() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut baseline = BTreeMap::new();
        baseline.insert("src/a.ts".to_string(), Some("one\n".to_string()));
        baseline.insert("src/b.ts".to_string(), None);
        persist_worktree_baseline(temp.path(), &WorkflowNodeRunId::new("node-1"), &baseline)
            .unwrap();
        let loaded: BTreeMap<String, Option<String>> =
            serde_json::from_slice(&std::fs::read(temp.path().join("node-1.json")).unwrap())
                .unwrap();
        assert_eq!(loaded, baseline);
    }

    /// Verifies the completion-time diff against a baseline loaded from its side file reports
    /// only the node's own changes since the node started.
    #[test]
    fn compute_file_changes_against_a_loaded_baseline() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut baseline = BTreeMap::new();
        baseline.insert("src/a.ts".to_string(), Some("one\n".to_string()));
        persist_worktree_baseline(temp.path(), &WorkflowNodeRunId::new("node-1"), &baseline)
            .unwrap();
        let loaded: BTreeMap<String, Option<String>> =
            serde_json::from_slice(&std::fs::read(temp.path().join("node-1.json")).unwrap())
                .unwrap();

        let mut current = baseline;
        current.insert("src/a.ts".to_string(), Some("one\ntwo\n".to_string()));
        current.insert("src/new.ts".to_string(), Some("fresh\n".to_string()));
        assert_eq!(
            compute_file_changes(Some(&loaded), Some(&current)),
            vec![
                FileChange {
                    path: "src/a.ts".to_string(),
                    additions: 1,
                    deletions: 0
                },
                FileChange {
                    path: "src/new.ts".to_string(),
                    additions: 1,
                    deletions: 0
                },
            ]
        );
    }

    /// Verifies the snapshot covers clean tracked files and files inside untracked directories,
    /// so the before/after delta reports the node's own edits rather than whole-file additions.
    #[test]
    fn capture_worktree_snapshot_diffs_clean_tracked_and_untracked_dir_files() {
        let scaffold = ora_test_support::GitTestScaffold::new("backend-workflow-snapshot")
            .expect("create Git test scaffold");
        let root = scaffold.repo_path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        // A tracked file that is clean at the baseline and modified by the node.
        std::fs::write(root.join("src/a.ts"), "one\ntwo\n").unwrap();
        scaffold
            .stage_all_and_commit("init")
            .expect("create snapshot baseline commit");

        let baseline = capture_worktree_snapshot(root).expect("capture baseline");
        // The clean tracked file is part of the baseline.
        assert_eq!(
            baseline.get("src/a.ts"),
            Some(&Some("one\ntwo\n".to_string()))
        );

        // The node edits the tracked file and creates files inside a new untracked directory.
        std::fs::write(root.join("src/a.ts"), "one\ntwo\nthree\n").unwrap();
        std::fs::create_dir_all(root.join("openspec/changes/demo")).unwrap();
        std::fs::write(root.join("openspec/changes/demo/proposal.md"), "fresh\n").unwrap();

        assert_eq!(
            compute_file_changes(Some(&baseline), capture_worktree_snapshot(root).as_ref()),
            vec![
                FileChange {
                    path: "openspec/changes/demo/proposal.md".to_string(),
                    additions: 1,
                    deletions: 0
                },
                FileChange {
                    path: "src/a.ts".to_string(),
                    additions: 1,
                    deletions: 0
                },
            ]
        );
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

    #[test]
    fn assistant_output_accumulator_keeps_only_assistant_text() {
        let mut accumulator = AssistantOutputAccumulator::default();
        accumulator.consume(&SessionUpdate::UserMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new("ignored")),
        )));
        accumulator.consume(&SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new("hello ")),
        )));
        accumulator.consume(&SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new("world")),
        )));
        assert_eq!(accumulator.into_output(), Some("hello world".to_string()));
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
