//! User-level workflow scenario coverage through the production Backend interface.
//!
//! Every scenario publishes an editor-shaped graph, supplies typed Start values the way the
//! run-start screen does, starts the run, and asserts the user-visible outcome: terminal
//! status, per-round agent sessions, rendered prompts, and the exposed variable pool. The
//! fake OpenCode agent echoes each rendered prompt back as its output, so prompt templates
//! double as data-flow assertions.

use super::{
    current_thread_runtime, install_fake_opencode_plugin, install_mcp_plugin, install_skill_plugin,
    open_ready_backend, seed_workspace,
};
use crate::setup::DesktopTestSetup;
use ora_backend::Backend;
use ora_contracts::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The agent CLI the fake plugin answers for; the model id is never validated by the fake.
const AGENT_CLI: &str = "official/ora-space.opencode";
/// Generous deadline: composite scenarios open one session per round through a real backend.
const RUN_DEADLINE: Duration = Duration::from_secs(20);

/// Polls until the Effect worker materializes one expected Skill file.
///
/// The Effect suite allows heavier plugin IPC and disk work than ordinary session readiness, so
/// this local deadline matches its 15-second budget without changing the shared readiness timeout.
fn wait_for_materialized_skill(path: &Path) -> TestResult {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if path.is_file() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(format!("skill package was not materialized at {}", path.display()).into())
}

/// One executed run together with the fake-agent journal for failure diagnostics.
struct ScenarioRun {
    detail: GetWorkflowRunResponse,
    journal: String,
    package_root: PathBuf,
}

impl ScenarioRun {
    fn status(&self) -> WorkflowRunStatus {
        self.detail.run.status
    }

    /// Every rendered prompt the fake agent served, as `(session_id, prompt)` pairs in served
    /// order. Prompt-level content (injected skill blocks, workspace boundaries) is asserted
    /// here because node outputs carry only the agent's final answer.
    fn prompts(&self) -> Vec<(String, String)> {
        std::fs::read_to_string(self.package_root.join("acp_prompts.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let entry: Value = serde_json::from_str(line).ok()?;
                Some((
                    entry["sessionId"].as_str()?.to_string(),
                    entry["prompt"].as_str()?.to_string(),
                ))
            })
            .collect()
    }

    /// The MCP server selections the fake agent recorded per session lifecycle call.
    fn mcp_calls(&self) -> Vec<Value> {
        std::fs::read_to_string(self.package_root.join("mcp_calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// Sessions opened for this run; one per executed agent round.
    fn sessions(&self) -> usize {
        self.detail
            .nodes
            .iter()
            .filter(|node| node.session_id.is_some())
            .count()
    }

    /// Outputs of every row recorded for one node id, in persisted order.
    fn node_outputs(&self, node_id: &str) -> Vec<&str> {
        self.detail
            .nodes
            .iter()
            .filter(|node| node.node_id == node_id)
            .filter_map(|node| node.output.as_deref())
            .collect()
    }

    /// Errors recorded on one node's rows.
    fn node_errors(&self, node_id: &str) -> Vec<&str> {
        self.detail
            .nodes
            .iter()
            .filter(|node| node.node_id == node_id)
            .filter_map(|node| node.error.as_deref())
            .collect()
    }

    /// Current value of one exposed root variable, e.g. (`iter`, `failed_count`).
    fn variable(&self, node_id: &str, root: &str) -> Option<&Value> {
        self.detail
            .variables
            .iter()
            .find(|variable| {
                variable
                    .selector
                    .first()
                    .is_some_and(|part| part == node_id)
                    && variable.selector.get(1).is_some_and(|part| part == root)
            })
            .and_then(|variable| variable.value.as_ref())
    }

    /// Node-level failure detail for assertion messages and CI diagnostics.
    fn failure_summary(&self) -> String {
        let nodes = self
            .detail
            .nodes
            .iter()
            .map(|node| {
                format!(
                    "{}({:?}{}{})",
                    node.node_id,
                    node.status,
                    node.error
                        .as_deref()
                        .map(|error| format!(", error: {error}"))
                        .unwrap_or_default(),
                    node.output
                        .as_deref()
                        .map(|output| format!(
                            ", output: {}",
                            output.chars().take(120).collect::<String>()
                        ))
                        .unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        format!(
            "status {:?}; run error {:?}; nodes [{}]; journal {}",
            self.detail.run.status, self.detail.run.error, nodes, self.journal
        )
    }

    /// The run's terminal output document, parsed as JSON.
    fn run_output(&self) -> Value {
        serde_json::from_str(
            self.detail
                .run
                .output
                .as_deref()
                .unwrap_or_else(|| panic!("run has no output; journal: {}", self.journal)),
        )
        .unwrap_or_else(|error| panic!("run output is not JSON: {error}"))
    }
}

/// Drives published graphs through the same production handlers the desktop UI calls.
struct WorkflowHarness {
    backend: Backend,
    package_root: PathBuf,
    workspace_id: String,
    workflow_seq: usize,
    /// Ids of graphs this harness published, newest last, so a scenario can create runs on a
    /// published graph while still observing the creation error itself.
    published_workflow_ids: Vec<String>,
    /// Kept alive so the sandbox's TempDir outlives the backend; dropping it first would delete
    /// the workspace directory out from under running sessions (Unix deletes eagerly).
    _setup: DesktopTestSetup,
}

impl WorkflowHarness {
    /// Opens a ready backend with the fake OpenCode agent installed.
    fn open() -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_plugins(|_| Ok(()))
    }

    /// Opens a ready backend after installing extra plugins beside the fake agent; plugins are
    /// written before the backend opens so boot discovery and skill projection pick them up.
    fn open_with_plugins(
        extra: impl FnOnce(&std::path::Path) -> Result<(), Box<dyn std::error::Error>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let setup = DesktopTestSetup::new()?;
        let home = setup.backend_paths().home_directory.clone();
        let package_root = install_fake_opencode_plugin(&home)?;
        extra(&home)?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        Ok(Self {
            backend,
            package_root,
            workspace_id,
            workflow_seq: 0,
            published_workflow_ids: Vec::new(),
            _setup: setup,
        })
    }

    /// The seeded main Workspace the Effect worker materializes Skills into.
    fn workspace_root(&self) -> PathBuf {
        self._setup.root().join("workspace")
    }

    /// Publishes the graph and drives one run to a terminal status with the given Start values.
    async fn run(
        &mut self,
        graph: Value,
        variables: BTreeMap<String, Value>,
    ) -> Result<ScenarioRun, Box<dyn std::error::Error>> {
        let run_id = self.pending_run(graph)?;
        self.run_on_pending(run_id, variables).await
    }

    /// Supplies values to an already-created pending run and drives it to a terminal status.
    async fn run_on_pending(
        &self,
        run_id: String,
        variables: BTreeMap<String, Value>,
    ) -> Result<ScenarioRun, Box<dyn std::error::Error>> {
        let result = self
            .backend
            .workflow_runs()
            .update_input(UpdateWorkflowRunInputRequest {
                run_id: run_id.clone(),
                input: None,
                variables,
            });
        result.map_err(|error| format!("update_input rejected valid values: {error}"))?;
        self.backend
            .workflow_runs()
            .start(StartWorkflowRunRequest {
                run_id: run_id.clone(),
            })?;
        self.wait_terminal(&run_id).await
    }

    /// Creates a pending run on a freshly published graph without providing values yet.
    fn pending_run(&mut self, graph: Value) -> Result<String, Box<dyn std::error::Error>> {
        self.pending_run_with_locale(graph, WorkflowRunLocale::EnUs)
    }

    /// Creates a pending run on a freshly published graph with an explicit display language.
    fn pending_run_with_locale(
        &mut self,
        graph: Value,
        locale: WorkflowRunLocale,
    ) -> Result<String, Box<dyn std::error::Error>> {
        self.publish(graph)?;
        self.create_pending_run_with_locale(locale)
            .map_err(|error| format!("run creation failed: {error}").into())
    }

    /// Publishes a graph as the next workflow version.
    fn publish(&mut self, graph: Value) -> Result<(), Box<dyn std::error::Error>> {
        self.workflow_seq += 1;
        let workflow = self
            .backend
            .workflows()
            .create(CreateWorkflowRequest {
                name: format!("Scenario {}", self.workflow_seq),
                graph: Some(graph.to_string()),
            })?
            .workflow;
        self.backend.workflows().publish(PublishWorkflowRequest {
            workflow_id: workflow.id.clone(),
            version: Some("v1".into()),
        })?;
        self.published_workflow_ids.push(workflow.id);
        Ok(())
    }

    /// Creates a pending run on the most recently published workflow.
    fn create_pending_run(&self) -> Result<String, ora_backend::BackendError> {
        self.create_pending_run_with_locale(WorkflowRunLocale::EnUs)
    }

    /// Creates a pending run whose prompt copy freezes the given display language.
    fn create_pending_run_with_locale(
        &self,
        locale: WorkflowRunLocale,
    ) -> Result<String, ora_backend::BackendError> {
        let workflow_id = self.published_workflow_ids.last().cloned().ok_or_else(|| {
            ora_backend::BackendError::new(
                ora_backend::ErrorClassification::NotFound,
                PublicError::WorkflowNotFound(EmptyErrorParams {}),
                "no published workflow",
            )
        })?;
        let run = self
            .backend
            .workflow_runs()
            .create(CreateWorkflowRunRequest {
                inject_last_failure: None,
                workspace_id: self.workspace_id.clone(),
                workflow_id,
                locale,
                snapshot_id: None,
                kickoff_input: None,
                name: None,
            })?
            .run;
        Ok(run.id)
    }

    /// Polls the most recently created run until it reaches a terminal status.
    async fn wait_terminal(&self, run_id: &str) -> Result<ScenarioRun, Box<dyn std::error::Error>> {
        let detail = self.poll(run_id).await?;
        let journal =
            std::fs::read_to_string(self.package_root.join("acp_calls.txt")).unwrap_or_default();
        Ok(ScenarioRun {
            detail,
            journal,
            package_root: self.package_root.clone(),
        })
    }

    /// Yields with `tokio::time::sleep` so the current-thread runtime keeps driving the
    /// backend's spawned session pumps between polls.
    async fn poll(
        &self,
        run_id: &str,
    ) -> Result<GetWorkflowRunResponse, Box<dyn std::error::Error>> {
        let deadline = tokio::time::Instant::now() + RUN_DEADLINE;
        loop {
            let detail = self.backend.workflow_runs().get(GetWorkflowRunRequest {
                run_id: run_id.into(),
            })?;
            if matches!(
                detail.run.status,
                WorkflowRunStatus::Succeeded | WorkflowRunStatus::Failed
            ) {
                return Ok(detail);
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "run did not settle within {RUN_DEADLINE:?}; status {:?}; nodes {:?}",
                detail.run.status,
                detail
                    .nodes
                    .iter()
                    .map(|node| (node.node_id.clone(), node.status))
                    .collect::<Vec<_>>(),
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

/// Agent-node data with the fake OpenCode executor; the prompt is echoed back on output.
fn agent_data(prompt: &str) -> Value {
    json!({"kind":"agent","agentConfig":{
        "executor":{"agentCli":AGENT_CLI,"modelId":"anthropic/claude-sonnet-4"},
        "prompt":prompt,"interactive":false
    }})
}

/// Agent-node data additionally owned by a Loop container.
fn loop_agent_data(prompt: &str, loop_id: &str) -> Value {
    let mut data = agent_data(prompt);
    data["containerId"] = json!(loop_id);
    data
}

/// Start-input declaration with an explicit field control and pool type.
fn input(name: &str, field_type: &str, value_type: &str) -> Value {
    json!({"name":name,"fieldType":field_type,"valueType":value_type})
}

// ── Start inputs: every supported data type ──

/// Every supported Start field type supplies a matching value that reaches the agent prompt
/// rendered per its pool type: strings verbatim, everything else as JSON.
#[test]
fn start_values_of_every_supported_type_reach_the_agent_prompt() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        input("text","text-input","string"),
                        input("count","number","number"),
                        input("flag","checkbox","boolean"),
                        {"name":"choice","fieldType":"select","valueType":"string","options":["x","y"]},
                        input("tags","json","array[string]"),
                        input("rows","json","array[object]"),
                        input("meta","json","object"),
                        input("raw","json","any")
                    ]}},
                    {"id":"agent","type":"workflow","position":{"x":360,"y":0},"data":agent_data(
                        "text={{#start.text#}} count={{#start.count#}} flag={{#start.flag#}} choice={{#start.choice#}} tags={{#start.tags#}} rows={{#start.rows#}} meta={{#start.meta#}} raw={{#start.raw#}}"
                    )},
                    {"id":"out","type":"workflow","position":{"x":720,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"echo","variableSelector":["agent","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"agent"},
                    {"source":"agent","target":"out"}
                ]
            });
            let run = harness.run(graph, BTreeMap::from([
                ("text".into(), json!("hello")),
                ("count".into(), json!(7)),
                ("flag".into(), json!(true)),
                ("choice".into(), json!("x")),
                ("tags".into(), json!(["a","b"])),
                ("rows".into(), json!([{"id":1}])),
                ("meta".into(), json!({"k":"v"})),
                ("raw".into(), json!(42)),
            ])).await?;

            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 1, "{}", run.failure_summary());
            let output = run.node_outputs("agent").join("\n");
            for fragment in [
                "text=hello",
                "count=7",
                "flag=true",
                "choice=x",
                "tags=[\"a\",\"b\"]",
                "rows=[{\"id\":1}]",
                "meta={\"k\":\"v\"}",
                "raw=42",
            ] {
                assert!(output.contains(fragment), "missing {fragment:?} in {output:?}");
            }
            assert_eq!(run.variable("start", "text"), Some(&json!("hello")));
            assert_eq!(run.variable("start", "count"), Some(&json!(7)));
            assert_eq!(run.variable("start", "tags"), Some(&json!(["a","b"])));
            Ok(())
        })
    })
}

/// The run-input screen's writes are validated against the declared pool types before the run
/// starts; a value of the wrong shape never enters the pool.
#[test]
fn run_inputs_are_validated_against_declared_types() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        input("count","number","number"),
                        input("meta","json","object"),
                        input("tags","json","array[string]"),
                        input("required_text","text-input","string")
                    ]}},
                    {"id":"agent","type":"workflow","position":{"x":360,"y":0},"data":agent_data("ok")},
                    {"id":"out","type":"workflow","position":{"x":720,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"echo","variableSelector":["agent","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"agent"},
                    {"source":"agent","target":"out"}
                ]
            });
            let run_id = harness.pending_run(graph)?;

            let cases: [(String, Value, &str, &str); 4] = [
                (
                    "count".into(),
                    json!("not-a-number"),
                    "count",
                    "value does not match the declared type number",
                ),
                (
                    "meta".into(),
                    json!([1, 2]),
                    "meta",
                    "value does not match the declared type object",
                ),
                (
                    "tags".into(),
                    json!({"a": 1}),
                    "tags",
                    "value does not match the declared type array[string]",
                ),
                (
                    "typo".into(),
                    json!("value"),
                    "typo",
                    "no declared Start variable has this name",
                ),
            ];
            for (name, value, expected_variable, expected_reason) in cases {
                let error = harness
                    .backend
                    .workflow_runs()
                    .update_input(UpdateWorkflowRunInputRequest {
                        run_id: run_id.clone(),
                        input: None,
                        variables: BTreeMap::from([(name.clone(), value)]),
                    })
                    .err()
                    .ok_or(format!("update_input accepted a mistyped value for {name}"))?;
                // The rejection must stay user-actionable: the classification is a bad request,
                // and the public error names the variable and the expected pool type instead of
                // a detail-free "application operation failed".
                assert_eq!(
                    error.classification(),
                    ora_backend::ErrorClassification::InvalidRequest,
                    "rejection for {name} must be a bad request, not internal: {error}"
                );
                assert_eq!(
                    error.public_error().clone(),
                    PublicError::WorkflowRunInputInvalid(WorkflowRunInputInvalidParams {
                        variable: expected_variable.to_string(),
                        reason: expected_reason.to_string(),
                    }),
                    "rejection for {name} must name the variable and reason"
                );
            }

            harness
                .backend
                .workflow_runs()
                .update_input(UpdateWorkflowRunInputRequest {
                    run_id: run_id.clone(),
                    input: None,
                    variables: BTreeMap::from([
                        ("count".into(), json!(3)),
                        ("meta".into(), json!({"k":"v"})),
                        ("tags".into(), json!(["a"])),
                        ("required_text".into(), json!("provided")),
                    ]),
                })
                .map_err(|error| format!("valid values were rejected: {error}"))?;
            harness
                .backend
                .workflow_runs()
                .start(StartWorkflowRunRequest { run_id: run_id.clone() })?;
            let run = harness.wait_terminal(&run_id).await?;
            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            Ok(())
        })
    })
}

/// Starting a run without a required Start value is rejected before any node executes.
#[test]
fn starting_without_a_required_start_value_is_rejected() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        {"name":"brief","fieldType":"text-input","valueType":"string","required":true}
                    ]}},
                    {"id":"agent","type":"workflow","position":{"x":360,"y":0},"data":agent_data("ok")},
                    {"id":"out","type":"workflow","position":{"x":720,"y":0},"data":{"kind":"output","outputs":[]}}
                ],
                "edges":[
                    {"source":"start","target":"agent"},
                    {"source":"agent","target":"out"}
                ]
            });
            let run_id = harness.pending_run(graph)?;
            let error = harness
                .backend
                .workflow_runs()
                .start(StartWorkflowRunRequest { run_id })
                .err()
                .ok_or("start accepted a run missing its required Start value")?;
            // The rejection names the missing variable so a user with several required values
            // does not have to guess which one is absent.
            assert_eq!(
                error.public_error().clone(),
                PublicError::WorkflowRunInputInvalid(WorkflowRunInputInvalidParams {
                    variable: "brief".to_string(),
                    reason: "required value is missing".to_string(),
                }),
                "start rejection must name the missing variable"
            );
            Ok(())
        })
    })
}

/// Starting a run with a select value outside its options is rejected naming the variable and
/// the option constraint, completing the start-validation rejection coverage.
#[test]
fn starting_with_an_out_of_option_start_value_is_rejected() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        {"name":"mode","fieldType":"select","valueType":"string","options":["fast","slow"]}
                    ]}},
                    {"id":"agent","type":"workflow","position":{"x":360,"y":0},"data":agent_data("ok")},
                    {"id":"out","type":"workflow","position":{"x":720,"y":0},"data":{"kind":"output","outputs":[]}}
                ],
                "edges":[
                    {"source":"start","target":"agent"},
                    {"source":"agent","target":"out"}
                ]
            });
            let run_id = harness.pending_run(graph)?;
            // The value is a well-typed string, so the input boundary accepts it; the option
            // constraint fires when the run starts, mirroring the editor's select control.
            harness
                .backend
                .workflow_runs()
                .update_input(UpdateWorkflowRunInputRequest {
                    run_id: run_id.clone(),
                    input: None,
                    variables: BTreeMap::from([("mode".into(), json!("turbo"))]),
                })
                .map_err(|error| format!("update_input rejected a well-typed option value: {error}"))?;
            let error = harness
                .backend
                .workflow_runs()
                .start(StartWorkflowRunRequest { run_id })
                .err()
                .ok_or("start accepted a value outside the select options")?;
            assert_eq!(
                error.public_error().clone(),
                PublicError::WorkflowRunInputInvalid(WorkflowRunInputInvalidParams {
                    variable: "mode".to_string(),
                    reason: "value is not one of the configured options".to_string(),
                }),
                "start rejection must name the out-of-option variable"
            );
            Ok(())
        })
    })
}

/// A zh-CN run renders the Chinese prompt copy and the fake agent must still answer with the
/// task instructions alone: the Chinese marker matches the assembler's half-width colon, so a
/// locale drift fails this exact-equality assertion instead of silently embedding the injected
/// context blocks into node outputs.
#[test]
fn zh_cn_runs_answer_with_the_task_instructions_only() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[]}},
                    {"id":"agent","type":"workflow","position":{"x":360,"y":0},"data":agent_data("请用一句话总结当前任务。")},
                    {"id":"out","type":"workflow","position":{"x":720,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"answer","variableSelector":["agent","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"agent"},
                    {"source":"agent","target":"out"}
                ]
            });
            let run_id =
                harness.pending_run_with_locale(graph, WorkflowRunLocale::ZhCn)?;
            let run = harness.run_on_pending(run_id, BTreeMap::new()).await?;
            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            // Exact equality pins both the marker match and the absence of injected blocks
            // (workspace boundary, workflow overview) in either locale.
            assert_eq!(
                run.node_outputs("agent").join("\n"),
                "Fake agent received: 请用一句话总结当前任务。"
            );
            let prompts = run
                .prompts()
                .iter()
                .map(|(_, prompt)| prompt.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                prompts.contains("任务要求:\n请用一句话总结当前任务。"),
                "zh-CN prompt must carry the Chinese task-instruction marker: {prompts:?}"
            );
            Ok(())
        })
    })
}

// ── Condition nodes ──

/// String equality routes to the matching branch; a non-matching value falls back to the
/// implicit else branch, and the inactive branch never opens a session.
#[test]
fn condition_routes_string_equality_and_falls_back_to_else() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        input("mode","text-input","string")
                    ]}},
                    {"id":"gate","type":"workflow","position":{"x":240,"y":0},"data":{"kind":"condition","cases":[
                        {"id":"fast","logic":"and","conditions":[
                            {"variableSelector":["start","mode"],"operator":"equals","value":"fast"}
                        ]}
                    ]}},
                    {"id":"fast-agent","type":"workflow","position":{"x":480,"y":-120},"data":agent_data("fast-branch")},
                    {"id":"slow-agent","type":"workflow","position":{"x":480,"y":120},"data":agent_data("slow-branch")},
                    {"id":"fast-out","type":"workflow","position":{"x":720,"y":-120},"data":{"kind":"output","outputs":[
                        {"name":"branch","variableSelector":["fast-agent","output"]}
                    ]}},
                    {"id":"slow-out","type":"workflow","position":{"x":720,"y":120},"data":{"kind":"output","outputs":[
                        {"name":"branch","variableSelector":["slow-agent","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"gate"},
                    {"source":"gate","sourceHandle":"fast","target":"fast-agent"},
                    {"source":"fast-agent","target":"fast-out"},
                    {"source":"gate","sourceHandle":"else","target":"slow-agent"},
                    {"source":"slow-agent","target":"slow-out"}
                ]
            });

            for (mode, expected_fragment, absent_fragment) in
                [("fast", "fast-branch", "slow-branch"), ("slow", "slow-branch", "fast-branch")]
            {
                let run = harness.run(graph.clone(), BTreeMap::from([("mode".into(), json!(mode))])).await?;
                assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
                assert_eq!(run.sessions(), 1, "{}", run.failure_summary());
                let outputs = format!(
                    "{}{}",
                    run.node_outputs("fast-agent").join("\n"),
                    run.node_outputs("slow-agent").join("\n"),
                );
                assert!(outputs.contains(expected_fragment), "{outputs:?}");
                assert!(!outputs.contains(absent_fragment), "{outputs:?}");
                let branch = run.run_output()["branch"].as_str().unwrap_or_default().to_string();
                assert!(
                    branch.contains("Fake agent received:") && branch.contains(expected_fragment),
                    "branch output: {branch:?}"
                );
            }
            Ok(())
        })
    })
}

/// Numeric and boolean operators combine under one case's logic, and the first matching case
/// wins when several could match: `big` and `small` deliberately overlap for `count > 3`, so
/// `(count=5, flag=true)` satisfies both and must resolve to `big-branch`.
#[test]
fn condition_compares_numbers_and_booleans_with_first_match_wins() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let base_graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        input("count","number","number"),
                        input("flag","checkbox","boolean")
                    ]}},
                    {"id":"gate","type":"workflow","position":{"x":240,"y":0},"data":{"kind":"condition","cases":[
                        {"id":"big","logic":"and","conditions":[
                            {"variableSelector":["start","count"],"operator":"greater_than","value":3},
                            {"variableSelector":["start","flag"],"operator":"is","value":true}
                        ]},
                        {"id":"small","logic":"or","conditions":[
                            {"variableSelector":["start","count"],"operator":"greater_than","value":3},
                            {"variableSelector":["start","count"],"operator":"equals","value":1}
                        ]}
                    ]}},
                    {"id":"big-agent","type":"workflow","position":{"x":480,"y":-140},"data":agent_data("big-branch")},
                    {"id":"small-agent","type":"workflow","position":{"x":480,"y":0},"data":agent_data("small-branch")},
                    {"id":"mid-agent","type":"workflow","position":{"x":480,"y":140},"data":agent_data("mid-branch")}
                ],
                "edges":[
                    {"source":"start","target":"gate"},
                    {"source":"gate","sourceHandle":"big","target":"big-agent"},
                    {"source":"gate","sourceHandle":"small","target":"small-agent"},
                    {"source":"gate","sourceHandle":"else","target":"mid-agent"}
                ]
            });
            let graph = |count: Value, flag: Value| {
                let mut graph = base_graph.clone();
                graph["nodes"][0]["data"]["inputVariables"] = json!([
                    {"name":"count","fieldType":"number","valueType":"number","value":count},
                    {"name":"flag","fieldType":"checkbox","valueType":"boolean","value":flag}
                ]);
                graph
            };
            for (count, flag, expected) in [
                // Both cases match (count > 3 with flag true): the first case must win.
                (json!(5), json!(true), "big-branch"),
                // Only `small` matches (count > 3 but flag false), proving the second case is
                // reachable instead of dead ordering.
                (json!(5), json!(false), "small-branch"),
                // Neither matches: the else fallback.
                (json!(3), json!(false), "mid-branch"),
                // The `or` fallback inside `small` (count == 1).
                (json!(1), json!(false), "small-branch"),
            ] {
                let run = harness.run(graph(count, flag), BTreeMap::new()).await?;
                assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
                assert_eq!(run.sessions(), 1, "{}", run.failure_summary());
                let outputs = format!(
                    "{}{}{}",
                    run.node_outputs("big-agent").join("\n"),
                    run.node_outputs("small-agent").join("\n"),
                    run.node_outputs("mid-agent").join("\n"),
                );
                assert!(outputs.contains(expected), "expected {expected:?} in {outputs:?}");
            }
            Ok(())
        })
    })
}

// ── Iteration nodes ──

/// Iterating an array of objects binds each round's `item` as an object: nested paths render
/// in prompts and gate Conditions can compare them.
#[test]
fn iteration_over_object_arrays_renders_nested_paths() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        input("rows","json","array[object]")
                    ]}},
                    {"id":"iter","type":"workflow","position":{"x":360,"y":0},"initialWidth":760,"initialHeight":420,"data":{"kind":"iteration","iterationConfig":{
                        "iteratorSelector":["start","rows"],"collectSelector":["body","output"],
                        "errorStrategy":"fail","maxIterations":10
                    }}},
                    {"id":"gate2","type":"workflow","parentId":"iter","position":{"x":420,"y":160},"data":{"kind":"condition","cases":[
                        {"id":"known","logic":"and","conditions":[
                            {"variableSelector":["iter","item","id"],"operator":"greater_than","value":0}
                        ]}
                    ]}},
                    {"id":"body","type":"workflow","parentId":"iter","position":{"x":660,"y":160},"data":agent_data(
                        "note={{#iter.item.note#}} item={{#iter.item#}}"
                    )},
                    {"id":"out","type":"workflow","position":{"x":1240,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"collected","variableSelector":["iter","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"iter"},
                    {"source":"iter","sourceHandle":"iteration-entry","target":"gate2"},
                    {"source":"gate2","sourceHandle":"known","target":"body"},
                    {"source":"iter","target":"out"}
                ]
            });
            let run = harness.run(graph, BTreeMap::from([(
                "rows".into(),
                json!([{"id":1,"note":"alpha"},{"id":2,"note":"beta"}]),
            )])).await?;

            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 2, "{}", run.failure_summary());
            let outputs = run.node_outputs("body").join("\n");
            assert!(outputs.contains("note=alpha"), "{outputs:?}");
            assert!(outputs.contains("note=beta"), "{outputs:?}");
            assert!(outputs.contains("item={\"id\":1,\"note\":\"alpha\"}"), "{outputs:?}");
            let collected = run.variable("iter", "output").cloned().unwrap_or_default();
            assert_eq!(collected.as_array().map(Vec::len), Some(2), "{collected:?}");
            assert_eq!(run.run_output()["collected"].as_array().map(Vec::len), Some(2));
            Ok(())
        })
    })
}

/// Under the `continue` strategy a round whose Condition bypasses the collect target settles
/// as failed: the run still succeeds, the failed round is counted, and only executed rounds
/// are collected.
#[test]
fn iteration_round_bypassing_the_collect_target_settles_failed_under_continue() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        input("rows","json","array[object]")
                    ]}},
                    {"id":"iter","type":"workflow","position":{"x":360,"y":0},"initialWidth":760,"initialHeight":420,"data":{"kind":"iteration","iterationConfig":{
                        "iteratorSelector":["start","rows"],"collectSelector":["body","output"],
                        "errorStrategy":"continue","maxIterations":10
                    }}},
                    {"id":"gate2","type":"workflow","parentId":"iter","position":{"x":420,"y":160},"data":{"kind":"condition","cases":[
                        {"id":"big","logic":"and","conditions":[
                            {"variableSelector":["iter","item","id"],"operator":"greater_than","value":1}
                        ]}
                    ]}},
                    {"id":"body","type":"workflow","parentId":"iter","position":{"x":660,"y":160},"data":agent_data("item={{#iter.item.note#}}")},
                    {"id":"out","type":"workflow","position":{"x":1240,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"collected","variableSelector":["iter","output"]},
                        {"name":"failed_count","variableSelector":["iter","failed_count"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"iter"},
                    {"source":"iter","sourceHandle":"iteration-entry","target":"gate2"},
                    {"source":"gate2","sourceHandle":"big","target":"body"},
                    {"source":"iter","target":"out"}
                ]
            });
            let run = harness.run(graph, BTreeMap::from([(
                "rows".into(),
                json!([{"id":1,"note":"alpha"},{"id":2,"note":"beta"}]),
            )])).await?;

            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 1, "only the id=2 round reaches the body; journal: {}", run.journal);
            assert_eq!(run.variable("iter", "failed_count"), Some(&json!(1)));
            let collected = run.variable("iter", "output").cloned().unwrap_or_default();
            assert_eq!(collected.as_array().map(Vec::len), Some(1), "{collected:?}");
            assert_eq!(run.run_output()["failed_count"], json!(1));
            Ok(())
        })
    })
}

/// An empty iterator source is legal: the iteration completes immediately with empty exposed
/// variables and no round ever opens a session.
#[test]
fn iteration_over_an_empty_array_completes_without_rounds() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        input("items","json","array[string]")
                    ]}},
                    {"id":"iter","type":"workflow","position":{"x":360,"y":0},"initialWidth":760,"initialHeight":420,"data":{"kind":"iteration","iterationConfig":{
                        "iteratorSelector":["start","items"],"collectSelector":["body","output"],
                        "errorStrategy":"fail","maxIterations":10
                    }}},
                    {"id":"body","type":"workflow","parentId":"iter","position":{"x":420,"y":160},"data":agent_data("item={{#iter.item#}}")},
                    {"id":"out","type":"workflow","position":{"x":1240,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"collected","variableSelector":["iter","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"iter"},
                    {"source":"iter","sourceHandle":"iteration-entry","target":"body"},
                    {"source":"iter","target":"out"}
                ]
            });
            let run = harness.run(graph, BTreeMap::from([("items".into(), json!([]))])).await?;

            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 0, "{}", run.failure_summary());
            assert_eq!(run.variable("iter", "output"), Some(&json!([])));
            assert_eq!(run.variable("iter", "failed_count"), Some(&json!(0)));
            assert_eq!(run.run_output()["collected"], json!([]));
            Ok(())
        })
    })
}

/// A source longer than maxIterations fails at the iteration's startup boundary before any
/// round executes, with an actionable error.
#[test]
fn iteration_fails_fast_when_the_source_exceeds_max_iterations() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        input("items","json","array[string]")
                    ]}},
                    {"id":"iter","type":"workflow","position":{"x":360,"y":0},"initialWidth":760,"initialHeight":420,"data":{"kind":"iteration","iterationConfig":{
                        "iteratorSelector":["start","items"],"collectSelector":["body","output"],
                        "errorStrategy":"fail","maxIterations":2
                    }}},
                    {"id":"body","type":"workflow","parentId":"iter","position":{"x":420,"y":160},"data":agent_data("item={{#iter.item#}}")},
                    {"id":"out","type":"workflow","position":{"x":1240,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"collected","variableSelector":["iter","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"iter"},
                    {"source":"iter","sourceHandle":"iteration-entry","target":"body"},
                    {"source":"iter","target":"out"}
                ]
            });
            let run = harness.run(graph, BTreeMap::from([(
                "items".into(),
                json!(["a", "b", "c"]),
            )])).await?;

            assert_eq!(run.status(), WorkflowRunStatus::Failed, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 0, "{}", run.failure_summary());
            let errors = run.node_errors("iter").join("\n");
            assert!(
                errors.contains("exceeding maxIterations"),
                "iteration error should explain the ceiling; got {errors:?}"
            );
            Ok(())
        })
    })
}

// ── Loop nodes ──

/// The loop exits as soon as its until condition holds, exposing the bound outputs.
#[test]
fn loop_exits_as_soon_as_the_until_condition_holds() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "schemaVersion": 2,
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[]}},
                    {"id":"loop","type":"workflow","position":{"x":360,"y":0},"data":{"kind":"loop","loopConfig":{
                        "maxIterations":3,
                        "variables":[],
                        "until":{"logic":"and","conditions":[
                            {"variableSelector":["writer","output"],"operator":"contains","value":"loop-write"}
                        ]},
                        "outputs":[{"name":"result","variableSelector":["writer","output"]}]
                    }}},
                    {"id":"entry","type":"workflow","parentId":"loop","position":{"x":420,"y":160},"data":{"kind":"start","containerId":"loop"}},
                    {"id":"writer","type":"workflow","parentId":"loop","position":{"x":660,"y":160},"data":loop_agent_data("loop-write","loop")},
                    {"id":"out","type":"workflow","position":{"x":960,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"result","variableSelector":["loop","result"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"loop"},
                    {"source":"entry","target":"writer"},
                    {"source":"loop","target":"out"}
                ]
            });
            let run = harness.run(graph, BTreeMap::new()).await?;

            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 1, "{}", run.failure_summary());
            let result = run
                .variable("loop", "result")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            assert!(result.contains("loop-write"), "loop result: {result:?}");
            let run_result = run.run_output()["result"].as_str().unwrap_or_default().to_string();
            assert!(run_result.contains("loop-write"), "run output: {run_result:?}");
            Ok(())
        })
    })
}

/// A loop that never satisfies its until condition fails at the round ceiling with the round
/// budget named, after exactly maxIterations sessions.
#[test]
fn loop_fails_at_the_round_ceiling_without_termination() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "schemaVersion": 2,
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[]}},
                    {"id":"loop","type":"workflow","position":{"x":360,"y":0},"data":{"kind":"loop","loopConfig":{
                        "maxIterations":2,
                        "variables":[],
                        "until":{"logic":"and","conditions":[
                            {"variableSelector":["writer","output"],"operator":"equals","value":"never-matches"}
                        ]},
                        "outputs":[{"name":"result","variableSelector":["writer","output"]}]
                    }}},
                    {"id":"entry","type":"workflow","parentId":"loop","position":{"x":420,"y":160},"data":{"kind":"start","containerId":"loop"}},
                    {"id":"writer","type":"workflow","parentId":"loop","position":{"x":660,"y":160},"data":loop_agent_data("loop-write","loop")},
                    {"id":"out","type":"workflow","position":{"x":960,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"result","variableSelector":["loop","result"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"loop"},
                    {"source":"entry","target":"writer"},
                    {"source":"loop","target":"out"}
                ]
            });
            let run = harness.run(graph, BTreeMap::new()).await?;

            assert_eq!(run.status(), WorkflowRunStatus::Failed, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 2, "{}", run.failure_summary());
            let errors = run.node_errors("loop").join("\n");
            assert!(
                errors.contains("did not terminate within 2 rounds"),
                "loop error should name the round budget; got {errors:?}"
            );
            Ok(())
        })
    })
}

/// Typed loop variables carry each round's feedback into the next round's prompt: round two
/// sees round one's output, and the until condition can detect it.
#[test]
fn loop_carries_typed_feedback_into_the_next_round() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "schemaVersion": 2,
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[]}},
                    {"id":"loop","type":"workflow","position":{"x":360,"y":0},"data":{"kind":"loop","loopConfig":{
                        "maxIterations":3,
                        "variables":[{"name":"draft","valueType":"string","initial":{"kind":"constant","value":""},"feedback":["writer","output"]}],
                        "until":{"logic":"and","conditions":[
                            {"variableSelector":["writer","output"],"operator":"contains","value":"draft=Fake agent"}
                        ]},
                        "outputs":[{"name":"result","variableSelector":["writer","output"]}]
                    }}},
                    {"id":"entry","type":"workflow","parentId":"loop","position":{"x":420,"y":160},"data":{"kind":"start","containerId":"loop"}},
                    {"id":"writer","type":"workflow","parentId":"loop","position":{"x":660,"y":160},"data":loop_agent_data("draft={{#loop.draft#}}","loop")},
                    {"id":"out","type":"workflow","position":{"x":960,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"result","variableSelector":["loop","result"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"loop"},
                    {"source":"entry","target":"writer"},
                    {"source":"loop","target":"out"}
                ]
            });
            let run = harness.run(graph, BTreeMap::new()).await?;

            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 2, "{}", run.failure_summary());
            let outputs = run.node_outputs("writer");
            assert_eq!(outputs.len(), 2, "{outputs:?}");
            assert!(
                !outputs[0].contains("draft=Fake agent"),
                "round one starts from the empty initial draft; got {:?}",
                outputs[0]
            );
            assert!(
                outputs[1].contains("draft=Fake agent received:"),
                "round two must embed round one's output; got {:?}",
                outputs[1]
            );
            assert_eq!(run.variable("loop", "result"), Some(&json!(outputs[1])));
            Ok(())
        })
    })
}

// ── Aggregator nodes ──

/// The aggregator passes the first assigned branch output through unchanged, whichever
/// mutually exclusive branch ran, and an assigned `false` is a value, not a missing output.
#[test]
fn aggregator_passes_the_first_assigned_branch_output() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let branch_graph = |choice: &str| {
                json!({
                    "nodes": [
                        {"id":"start","type":"workflow","position":{"x":0,"y":160},"data":{"kind":"start","inputVariables":[]}},
                        {"id":"condition","type":"workflow","position":{"x":180,"y":160},"data":{"kind":"condition","cases":[
                            {"id":"a","logic":"and","conditions":[
                                {"variableSelector":["route","choice"],"operator":"is","value":"a"}
                            ]}
                        ]}},
                        {"id":"a","type":"workflow","position":{"x":360,"y":80},"data":agent_data("a-branch-work")},
                        {"id":"b","type":"workflow","position":{"x":360,"y":260},"data":agent_data("b-branch-work")},
                        {"id":"aggregator","type":"workflow","position":{"x":560,"y":160},"data":{"kind":"aggregator","aggregatorConfig":{
                            "variables":[["a","output"],["b","output"]]
                        }}},
                        {"id":"out","type":"workflow","position":{"x":760,"y":160},"data":{"kind":"output","outputs":[
                            {"name":"result","variableSelector":["aggregator","output"]}
                        ]}}
                    ],
                    "edges":[
                        {"source":"start","target":"condition"},
                        {"source":"condition","sourceHandle":"a","target":"a"},
                        {"source":"condition","sourceHandle":"else","target":"b"},
                        {"source":"a","target":"aggregator"},
                        {"source":"b","target":"aggregator"},
                        {"source":"aggregator","target":"out"}
                    ],
                    "globalVariables":[{"name":"route.choice","valueType":"string","value":choice}]
                })
            };
            for (choice, expected_fragment) in [("a", "a-branch-work"), ("b", "b-branch-work")] {
                let run = harness.run(branch_graph(choice), BTreeMap::new()).await?;
                assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
                assert_eq!(run.sessions(), 1, "{}", run.failure_summary());
                let aggregated = run
                    .variable("aggregator", "output")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                assert!(aggregated.contains(expected_fragment), "aggregated: {aggregated:?}");
                let run_result = run.run_output()["result"].as_str().unwrap_or_default().to_string();
                assert!(run_result.contains(expected_fragment), "run output: {run_result:?}");
            }

            let false_graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[]}},
                    {"id":"aggregator","type":"workflow","position":{"x":360,"y":0},"data":{"kind":"aggregator","aggregatorConfig":{
                        "variables":[["flag","value"]]
                    }}},
                    {"id":"out","type":"workflow","position":{"x":720,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"result","variableSelector":["aggregator","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"aggregator"},
                    {"source":"aggregator","target":"out"}
                ],
                "globalVariables":[{"name":"flag.value","valueType":"boolean","value":false}]
            });
            let run = harness.run(false_graph, BTreeMap::new()).await?;
            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.variable("aggregator", "output"), Some(&json!(false)));
            assert_eq!(run.run_output()["result"], json!(false));
            Ok(())
        })
    })
}

// ── Agent output semantics: the node output is the agent's final answer ──

/// Conditions and downstream bindings see exactly the agent's final answer: the default echo
/// answers with the task text (no injected workspace or workflow-context blocks), and a
/// `FAKE_REPLY:` directive answers with exact text, so equality-based routing and matching work
/// the way a real agent's answers would.
#[test]
fn conditions_route_on_the_exact_agent_output_without_injected_context() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[]}},
                    {"id":"echo","type":"workflow","position":{"x":200,"y":0},"data":agent_data("review the code")},
                    {"id":"judge","type":"workflow","position":{"x":440,"y":0},"data":agent_data("FAKE_REPLY: approved")},
                    {"id":"gate","type":"workflow","position":{"x":680,"y":0},"data":{"kind":"condition","cases":[
                        {"id":"approved","logic":"and","conditions":[
                            {"variableSelector":["judge","output"],"operator":"equals","value":"approved"}
                        ]}
                    ]}},
                    {"id":"pass","type":"workflow","position":{"x":920,"y":-100},"data":agent_data("FAKE_REPLY: gate passed")},
                    {"id":"fail","type":"workflow","position":{"x":920,"y":100},"data":agent_data("FAKE_REPLY: gate rejected")},
                    {"id":"out","type":"workflow","position":{"x":1160,"y":-100},"data":{"kind":"output","outputs":[
                        {"name":"echo_text","variableSelector":["echo","output"]},
                        {"name":"answer","variableSelector":["pass","output"]}
                    ]}},
                    {"id":"out2","type":"workflow","position":{"x":1160,"y":100},"data":{"kind":"output","outputs":[
                        {"name":"answer","variableSelector":["fail","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"echo"},
                    {"source":"echo","target":"judge"},
                    {"source":"judge","target":"gate"},
                    {"source":"gate","sourceHandle":"approved","target":"pass"},
                    {"source":"gate","sourceHandle":"else","target":"fail"},
                    {"source":"pass","target":"out"},
                    {"source":"fail","target":"out2"}
                ]
            });
            let run = harness.run(graph, BTreeMap::new()).await?;

            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 3, "{}", run.failure_summary());
            // The default answer is the task text alone: injected context blocks never reach the
            // output, the variable pool, or downstream bindings.
            assert_eq!(
                run.node_outputs("echo"),
                vec!["Fake agent received: review the code"],
                "the node output must be the agent's final answer without injected context"
            );
            let echo_output = run.variable("echo", "output").cloned().unwrap_or_default();
            assert_eq!(echo_output, json!("Fake agent received: review the code"));
            for marker in ["<workspace_root>", "<current_workflow_step>", "<workflow_context>"] {
                assert!(
                    !run.node_outputs("echo").join("").contains(marker),
                    "injected block {marker} leaked into the output"
                );
            }
            // The directive answer is exact, so equality routing works on agent outputs.
            assert_eq!(run.node_outputs("judge"), vec!["approved"]);
            assert_eq!(run.node_outputs("fail"), Vec::<&str>::new(), "else branch must not run");
            assert_eq!(run.node_outputs("pass"), vec!["gate passed"]);
            let output = run.run_output();
            assert_eq!(output["echo_text"], json!("Fake agent received: review the code"));
            assert_eq!(output["answer"], json!("gate passed"));
            Ok(())
        })
    })
}

/// A Loop whose until condition compares the agent output for equality exits on the first
/// matching round; feedback variables carry only the answer, not the injected prompt context.
#[test]
fn loop_until_equals_matches_the_exact_agent_output() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "schemaVersion": 2,
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[]}},
                    {"id":"loop","type":"workflow","position":{"x":360,"y":0},"data":{"kind":"loop","loopConfig":{
                        "maxIterations":3,
                        "variables":[{"name":"draft","valueType":"string","initial":{"kind":"constant","value":""},"feedback":["writer","output"]}],
                        "until":{"logic":"and","conditions":[
                            {"variableSelector":["writer","output"],"operator":"equals","value":"done"}
                        ]},
                        "outputs":[{"name":"result","variableSelector":["writer","output"]}]
                    }}},
                    {"id":"entry","type":"workflow","parentId":"loop","position":{"x":420,"y":160},"data":{"kind":"start","containerId":"loop"}},
                    {"id":"writer","type":"workflow","parentId":"loop","position":{"x":660,"y":160},"data":loop_agent_data("FAKE_REPLY: done","loop")},
                    {"id":"out","type":"workflow","position":{"x":960,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"result","variableSelector":["loop","result"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"loop"},
                    {"source":"entry","target":"writer"},
                    {"source":"loop","target":"out"}
                ]
            });
            let run = harness.run(graph, BTreeMap::new()).await?;

            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 1, "the exact answer must satisfy until on round one; journal: {}", run.journal);
            assert_eq!(run.variable("loop", "result"), Some(&json!("done")));
            assert_eq!(run.run_output()["result"], json!("done"));
            Ok(())
        })
    })
}

// ── Combined pipeline: Condition + Iteration + Loop over typed Start inputs ──

/// The composed pipeline chains every composite control over typed Start inputs: a Condition
/// gate picks the processing branch, an Iteration region filters per-item work under the
/// `continue` strategy, and a Loop refines its draft with typed feedback until its until
/// condition holds. The skipped mode exercises the gate's else path without any session.
#[test]
fn combined_condition_iteration_loop_pipeline() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "schemaVersion": 2,
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        input("mode","text-input","string"),
                        input("threshold","number","number"),
                        input("items","json","array[string]"),
                        input("meta","json","object")
                    ]}},
                    {"id":"gate","type":"workflow","position":{"x":240,"y":0},"data":{"kind":"condition","cases":[
                        {"id":"run","logic":"and","conditions":[
                            {"variableSelector":["start","mode"],"operator":"equals","value":"run"},
                            {"variableSelector":["start","threshold"],"operator":"greater_than_or_equal","value":2}
                        ]}
                    ]}},
                    {"id":"iter","type":"workflow","position":{"x":480,"y":0},"initialWidth":760,"initialHeight":420,"data":{"kind":"iteration","iterationConfig":{
                        "iteratorSelector":["start","items"],"collectSelector":["body","output"],
                        "errorStrategy":"continue","maxIterations":10
                    }}},
                    {"id":"gate2","type":"workflow","parentId":"iter","position":{"x":540,"y":160},"data":{"kind":"condition","cases":[
                        {"id":"go","logic":"and","conditions":[
                            {"variableSelector":["iter","item"],"operator":"not_equals","value":"beta"}
                        ]}
                    ]}},
                    {"id":"body","type":"workflow","parentId":"iter","position":{"x":780,"y":160},"data":agent_data("item={{#iter.item#}} index={{#iter.index#}}")},
                    {"id":"loop","type":"workflow","position":{"x":1320,"y":0},"data":{"kind":"loop","loopConfig":{
                        "maxIterations":3,
                        "variables":[{"name":"draft","valueType":"string","initial":{"kind":"constant","value":""},"feedback":["writer","output"]}],
                        "until":{"logic":"and","conditions":[
                            {"variableSelector":["writer","output"],"operator":"contains","value":"draft=Fake agent"}
                        ]},
                        "outputs":[{"name":"result","variableSelector":["writer","output"]}]
                    }}},
                    {"id":"entry","type":"workflow","parentId":"loop","position":{"x":1380,"y":160},"data":{"kind":"start","containerId":"loop"}},
                    {"id":"writer","type":"workflow","parentId":"loop","position":{"x":1620,"y":160},"data":loop_agent_data(
                        "team={{#start.meta#}} collected={{#iter.output#}} draft={{#loop.draft#}}","loop"
                    )},
                    {"id":"out","type":"workflow","position":{"x":1900,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"collected","variableSelector":["iter","output"]},
                        {"name":"failed_count","variableSelector":["iter","failed_count"]},
                        {"name":"loop_result","variableSelector":["loop","result"]}
                    ]}},
                    {"id":"out2","type":"workflow","position":{"x":1900,"y":260},"data":{"kind":"output","outputs":[
                        {"name":"skipped","variableSelector":["start","mode"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"gate"},
                    {"source":"gate","sourceHandle":"run","target":"iter"},
                    {"source":"gate","sourceHandle":"else","target":"out2"},
                    {"source":"iter","sourceHandle":"iteration-entry","target":"gate2"},
                    {"source":"gate2","sourceHandle":"go","target":"body"},
                    {"source":"iter","target":"loop"},
                    {"source":"entry","target":"writer"},
                    {"source":"loop","target":"out"}
                ]
            });

            let run = harness.run(graph.clone(), BTreeMap::from([
                ("mode".into(), json!("run")),
                ("threshold".into(), json!(2)),
                ("items".into(), json!(["alpha", "beta", "gamma"])),
                ("meta".into(), json!({"team": "ora"})),
            ])).await?;
            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            // Two iteration rounds reach the body (beta is gated away) plus two loop rounds.
            assert_eq!(run.sessions(), 4, "{}", run.failure_summary());
            let body_outputs = run.node_outputs("body").join("\n");
            assert!(body_outputs.contains("item=alpha index=0"), "{body_outputs:?}");
            assert!(body_outputs.contains("item=gamma index=2"), "{body_outputs:?}");
            assert!(!body_outputs.contains("item=beta"), "{body_outputs:?}");
            assert_eq!(run.variable("iter", "failed_count"), Some(&json!(1)));
            let collected = run.variable("iter", "output").cloned().unwrap_or_default();
            assert_eq!(collected.as_array().map(Vec::len), Some(2), "{collected:?}");
            let writer_outputs = run.node_outputs("writer");
            assert_eq!(writer_outputs.len(), 2, "{writer_outputs:?}");
            assert!(
                writer_outputs[1].contains("team={\"team\":\"ora\"}"),
                "the loop must see the object Start value; got {:?}",
                writer_outputs[1]
            );
            assert!(
                writer_outputs[1].contains("collected=[\"Fake agent received:"),
                "the loop must see the iteration's collected output; got {:?}",
                writer_outputs[1]
            );
            assert!(
                writer_outputs[1].contains("item=alpha index=0")
                    && writer_outputs[1].contains("item=gamma index=2"),
                "the collected array must hold both gated-in rounds; got {:?}",
                writer_outputs[1]
            );
            assert!(
                writer_outputs[1].contains("draft=Fake agent received:"),
                "round two must embed round one's output; got {:?}",
                writer_outputs[1]
            );
            assert_eq!(run.variable("loop", "result"), Some(&json!(writer_outputs[1])));
            assert_eq!(run.run_output()["failed_count"], json!(1));
            assert_eq!(run.run_output()["collected"].as_array().map(Vec::len), Some(2));

            let skipped = harness.run(graph, BTreeMap::from([
                ("mode".into(), json!("skip")),
                ("threshold".into(), json!(0)),
                ("items".into(), json!([])),
                ("meta".into(), json!({"team": "ora"})),
            ])).await?;
            assert_eq!(skipped.status(), WorkflowRunStatus::Succeeded, "{}", skipped.failure_summary());
            assert_eq!(skipped.sessions(), 0, "{}", skipped.failure_summary());
            assert_eq!(skipped.run_output()["skipped"], json!("skip"));
            Ok(())
        })
    })
}

/// File-typed Start values reach the prompt as canonical workspace-relative references and
/// unsafe paths are rejected at the run-input boundary before the run starts.
#[test]
fn file_typed_start_values_render_as_references_and_reject_unsafe_paths() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        input("source","file","file"),
                        input("attachments","file-list","array[file]")
                    ]}},
                    {"id":"agent","type":"workflow","position":{"x":360,"y":0},"data":agent_data(
                        "source={{#start.source#}} attachments={{#start.attachments#}}"
                    )},
                    {"id":"out","type":"workflow","position":{"x":720,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"echo","variableSelector":["agent","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"agent"},
                    {"source":"agent","target":"out"}
                ]
            });
            let run_id = harness.pending_run(graph)?;
            for (name, value) in [
                ("source", json!("C:/absolute/secret.txt")),
                ("source", json!("../escape.txt")),
                ("attachments", json!(["ok.txt", "..\\escape.txt"])),
            ] {
                let error = harness
                    .backend
                    .workflow_runs()
                    .update_input(UpdateWorkflowRunInputRequest {
                        run_id: run_id.clone(),
                        input: None,
                        variables: BTreeMap::from([(name.into(), value)]),
                    })
                    .err()
                    .ok_or(format!("update_input accepted the unsafe file path for {name}"))?;
                // Unsafe paths are ordinary type rejections: a bad request naming the variable,
                // not an internal failure.
                assert_eq!(
                    error.classification(),
                    ora_backend::ErrorClassification::InvalidRequest,
                    "unsafe path rejection for {name} must be a bad request: {error}"
                );
                assert!(
                    matches!(
                        error.public_error(),
                        PublicError::WorkflowRunInputInvalid(
                            WorkflowRunInputInvalidParams { variable, .. }
                        ) if variable == name
                    ),
                    "unsafe path rejection for {name} must name the variable"
                );
            }

            let run = harness
                .run_on_pending(
                    run_id,
                    BTreeMap::from([
                        (
                            "source".into(),
                            json!({"kind":"workspace_file","path":"docs/input.txt"}),
                        ),
                        (
                            "attachments".into(),
                            json!([
                                {"kind":"workspace_file","path":"one.txt"},
                                {"kind":"workspace_file","path":"nested/two.txt"}
                            ]),
                        ),
                    ]),
                )
                .await?;
            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 1, "{}", run.failure_summary());
            let output = run.node_outputs("agent").join("\n");
            assert!(
                output.contains("source={\"kind\":\"workspace_file\",\"path\":\"docs/input.txt\"}"),
                "{output:?}"
            );
            assert!(
                output.contains("attachments=[{\"kind\":\"workspace_file\",\"path\":\"one.txt\"},{\"kind\":\"workspace_file\",\"path\":\"nested/two.txt\"}]"),
                "{output:?}"
            );
            Ok(())
        })
    })
}

// ── Skill plugins inside workflow agent nodes ──

/// An installed Skill plugin contributes a skill the run can bind: deployment freezes the
/// binding into the materialization receipt, the agent's prompt opens with the mandatory
/// `/name` invocation contract naming the materialized worktree copy, and the node output
/// stays the agent's answer. The on-disk directory keeps the Skill's canonical `_` while the
/// prompt's `/name` spelling normalizes it to `-`. Disabled bindings remain portable without
/// blocking deployment, while an enabled binding to a skill that does not exist fails run
/// creation actionably.
#[test]
fn skill_plugins_bound_to_agent_nodes_reach_the_prompt() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open_with_plugins(|home| {
                install_skill_plugin(home, "review-pack", "code_review")?;
                Ok(())
            })?;
            let materialized_skill = harness
                .workspace_root()
                .join(".opencode")
                .join("skills")
                .join("code_review")
                .join("SKILL.md");
            wait_for_materialized_skill(&materialized_skill)?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[]}},
                    {"id":"reviewer","type":"workflow","position":{"x":360,"y":0},"data":{"kind":"agent","agentConfig":{
                        "executor":{"agentCli":AGENT_CLI,"modelId":"anthropic/claude-sonnet-4"},
                        "prompt":"FAKE_REPLY: reviewed",
                        "interactive":false,
                        "skills":[
                            {"skillId":"code_review","enabled":true},
                            {"skillId":"nonexistent_skill","enabled":false}
                        ]
                    }}},
                    {"id":"out","type":"workflow","position":{"x":720,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"answer","variableSelector":["reviewer","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"reviewer"},
                    {"source":"reviewer","target":"out"}
                ]
            });
            let run = harness.run(graph.clone(), BTreeMap::new()).await?;

            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 1, "{}", run.failure_summary());
            // The invocation contract is prompt-level content: it reaches the agent's session,
            // never the node output or the variable pool.
            let prompts = run.prompts();
            assert_eq!(prompts.len(), 1, "one session, one prompt; journal: {}", run.journal);
            let prompt = &prompts[0].1;
            assert!(prompt.contains("<required_skills>"), "missing skill block: {prompt:?}");
            assert!(
                prompt.contains("/code-review"),
                "missing invocation: {prompt:?}"
            );
            // The discovery root is the agent plugin's declared Effect Resource
            // (`.opencode/skills` for this fixture), not a host-wide convention. The prompt names
            // the directory Effect materialized, not the Agent invocation spelling.
            assert!(
                prompt.contains(".opencode/skills/code_review"),
                "the prompt must name the materialized worktree copy: {prompt:?}"
            );
            assert!(
                !prompt.contains(".opencode/skills/code-review"),
                "the prompt must not substitute the invocation spelling for the disk identity: {prompt:?}"
            );
            assert!(
                materialized_skill.is_file(),
                "the prompt path must exist in the worktree: {materialized_skill:?}"
            );
            assert!(
                !prompt.contains("nonexistent_skill"),
                "disabled bindings must stay out of the prompt: {prompt:?}"
            );
            assert_eq!(run.node_outputs("reviewer"), vec!["reviewed"]);
            assert_eq!(run.run_output()["answer"], json!("reviewed"));

            // An enabled binding to a missing skill fails run creation with a named error
            // instead of failing later during execution.
            let mut broken = graph.clone();
            broken["nodes"][1]["data"]["agentConfig"]["skills"] =
                json!([{"skillId":"nonexistent_skill","enabled":true}]);
            harness.publish(broken)?;
            let error = harness
                .create_pending_run()
                .err()
                .ok_or("run creation accepted an enabled binding to a missing skill")?;
            assert_eq!(
                error.public_error().clone(),
                PublicError::WorkflowSkillNotFound(EmptyErrorParams {}),
                "missing skill must fail run creation with the named public error"
            );
            Ok(())
        })
    })
}

// ── MCP plugins inside workflow agent nodes ──

/// An installed MCP plugin reaches the run's agent sessions only through the node's frozen
/// allowlist: enabled bindings deliver exactly their server, disabled bindings deliver nothing,
/// and every round of an iteration region gets the same frozen selection.
#[test]
fn mcp_plugins_bound_to_agent_nodes_reach_only_the_frozen_allowlist() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open_with_plugins(|home| {
                install_mcp_plugin(home, "tools")?;
                install_mcp_plugin(home, "other")?;
                Ok(())
            })?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        input("items","json","array[string]")
                    ]}},
                    {"id":"iter","type":"workflow","position":{"x":360,"y":0},"initialWidth":760,"initialHeight":420,"data":{"kind":"iteration","iterationConfig":{
                        "iteratorSelector":["start","items"],"collectSelector":["body","output"],
                        "errorStrategy":"fail","maxIterations":10
                    }}},
                    {"id":"body","type":"workflow","parentId":"iter","position":{"x":420,"y":160},"data":{"kind":"agent","agentConfig":{
                        "executor":{"agentCli":AGENT_CLI,"modelId":"anthropic/claude-sonnet-4"},
                        "prompt":"item={{#iter.item#}}",
                        "interactive":false,
                        "mcps":[
                            {"mcpId":"official/tools","enabled":true},
                            {"mcpId":"official/other","enabled":false}
                        ]
                    }}},
                    {"id":"out","type":"workflow","position":{"x":1240,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"collected","variableSelector":["iter","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"iter"},
                    {"source":"iter","sourceHandle":"iteration-entry","target":"body"},
                    {"source":"iter","target":"out"}
                ]
            });
            let run = harness.run(graph, BTreeMap::from([("items".into(), json!(["a", "b"]))])).await?;

            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 2, "{}", run.failure_summary());
            let calls = run.mcp_calls();
            let session_news: Vec<&Value> = calls
                .iter()
                .filter(|call| call["method"] == "session/new")
                .collect();
            assert_eq!(
                session_news.len(),
                2,
                "each round opens its own session; calls: {calls:?}"
            );
            for call in &session_news {
                assert_eq!(
                    call["servers"], json!(["official/tools", "ora-workflow-completion"]),
                    "the frozen allowlist must deliver only the enabled MCP; calls: {calls:?}"
                );
            }
            assert!(
                !calls.iter().any(|call| call["servers"] == json!(["official/other"])),
                "disabled MCP bindings must never reach a session; calls: {calls:?}"
            );
            let collected = run.variable("iter", "output").cloned().unwrap_or_default();
            assert_eq!(
                collected,
                json!(["Fake agent received: item=a", "Fake agent received: item=b"])
            );
            Ok(())
        })
    })
}

// ── Structured agent outputs feeding iteration collection ──

/// A structured-output agent inside an iteration answers typed JSON: the contract parses the
/// final answer into the node's structured variable, the collect target gathers one object per
/// round into a typed array of objects, and the run output carries the objects unchanged.
#[test]
fn structured_outputs_collect_into_typed_object_arrays() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let mut harness = WorkflowHarness::open()?;
            let graph = json!({
                "nodes": [
                    {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
                        input("items","json","array[string]")
                    ]}},
                    {"id":"iter","type":"workflow","position":{"x":360,"y":0},"initialWidth":760,"initialHeight":420,"data":{"kind":"iteration","iterationConfig":{
                        "iteratorSelector":["start","items"],"collectSelector":["body","structured_output"],
                        "errorStrategy":"fail","maxIterations":10
                    }}},
                    {"id":"body","type":"workflow","parentId":"iter","position":{"x":420,"y":160},"data":{"kind":"agent","agentConfig":{
                        "executor":{"agentCli":AGENT_CLI,"modelId":"anthropic/claude-sonnet-4"},
                        "prompt":"FAKE_REPLY: {\"item\":\"{{#iter.item#}}\",\"score\":7,\"tags\":[\"keep\"]}",
                        "interactive":false,
                        "outputContract":{
                            "type":"structured",
                            "schema":{
                                "type":"object",
                                "properties":{
                                    "item":{"type":"string"},
                                    "score":{"type":"number"},
                                    "tags":{"type":"array[string]"}
                                },
                                "required":["item","score","tags"],
                                "additionalProperties":false
                            }
                        }
                    }}},
                    {"id":"out","type":"workflow","position":{"x":1240,"y":0},"data":{"kind":"output","outputs":[
                        {"name":"collected","variableSelector":["iter","output"]}
                    ]}}
                ],
                "edges":[
                    {"source":"start","target":"iter"},
                    {"source":"iter","sourceHandle":"iteration-entry","target":"body"},
                    {"source":"iter","target":"out"}
                ]
            });
            let run = harness
                .run(graph, BTreeMap::from([("items".into(), json!(["alpha", "beta"]))]))
                .await?;

            assert_eq!(run.status(), WorkflowRunStatus::Succeeded, "{}", run.failure_summary());
            assert_eq!(run.sessions(), 2, "{}", run.failure_summary());
            let expected = json!([
                {"item":"alpha","score":7,"tags":["keep"]},
                {"item":"beta","score":7,"tags":["keep"]}
            ]);
            assert_eq!(run.variable("iter", "output"), Some(&expected));
            assert_eq!(
                run.variable("body", "structured_output"),
                Some(&json!({"item":"beta","score":7,"tags":["keep"]}))
            );
            assert_eq!(run.run_output()["collected"], expected);
            Ok(())
        })
    })
}
