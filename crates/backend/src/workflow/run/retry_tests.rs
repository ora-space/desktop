//! Automatic retry of failed agent attempts through the production engine and real SQLite.
//!
//! The engine runs on a hand-moved clock and a recording timer, so every wait is asserted to the
//! millisecond and every wake is driven explicitly. Composite scopes (Iteration, Loop) and the
//! boot sweep are covered in `retry_composite_tests`; the tokio timer in `retry_timer`.

use super::last_failure::previous_failure_for_injection;
use super::prompt::{WorkflowPromptRequest, assemble_workflow_prompt};
use super::test_fixture::{SeqGen, bootstrap, seeded_pending_run};
use agent_client_protocol_schema::v1::ContentBlock;
use ora_application::{
    CancelWorkflowRunResult, Clock, ExecutionContext, GetWorkflowRunHandler, NodeAutoRetry,
    NodeExecutor, NodeFailure, NodeFailureKind, NodeRetryWait, RETRY_WAIT_KEY,
    ResumeWorkflowRunResult, WorkflowGraph, WorkflowGraphNode, WorkflowRetryTimer,
    WorkflowRunEngine, WorkflowRunEngineRepository, WorkflowRunPayload, WorkflowRunRepository,
    WorkflowVariablePool,
};
use ora_contracts::{GetWorkflowRunRequest, WorkflowNodeFailedAttempt};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository};
use ora_domain::{
    WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRun, WorkflowRunDetail,
    WorkflowRunId, WorkflowRunStatus, WorkflowScopeId,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// A clock the test moves by hand; the engine and the assertions share it.
#[derive(Clone, Default)]
pub(super) struct ManualClock(Arc<AtomicI64>);

impl ManualClock {
    fn set(&self, now: i64) {
        self.0.store(now, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_timestamp_millis(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Records every deadline the engine arms instead of sleeping.
#[derive(Default)]
pub(super) struct RecordingTimer {
    armed: Mutex<Vec<(String, i64)>>,
}

impl WorkflowRetryTimer for RecordingTimer {
    fn arm(&self, _run_id: &WorkflowRunId, node_run_id: &WorkflowNodeRunId, due_at: i64) {
        self.armed
            .lock()
            .unwrap()
            .push((node_run_id.to_string(), due_at));
    }
}

/// One dispatch exactly as the Agent runtime hands it to the session executor.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Dispatch {
    pub(super) node_run_id: String,
    pub(super) node_id: String,
    pub(super) scope_id: WorkflowScopeId,
    /// Node ids of the graph the dispatch was resolved against (outer graph or a Loop body).
    pub(super) graph_nodes: Vec<String>,
    pub(super) pool_values: BTreeMap<String, Value>,
}

/// Records every dispatch while leaving completion under the test's control.
#[derive(Clone, Default)]
pub(super) struct DispatchLog(pub(super) Arc<Mutex<Vec<Dispatch>>>);

impl NodeExecutor for DispatchLog {
    fn dispatch(
        &self,
        node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        graph: &WorkflowGraph,
        _context: &ExecutionContext,
        scope_id: &ora_domain::WorkflowScopeId,
        pool: &WorkflowVariablePool,
    ) {
        let mut graph_nodes: Vec<String> = graph.nodes().map(|node| node.id.clone()).collect();
        graph_nodes.sort();
        self.0.lock().unwrap().push(Dispatch {
            node_run_id: node_run_id.to_string(),
            node_id: node.id.clone(),
            scope_id: scope_id.clone(),
            graph_nodes,
            pool_values: pool.values.clone(),
        });
    }
}

pub(super) type RetryEngine =
    WorkflowRunEngine<SqliteWorkflowRunEngineRepository, SeqGen, ManualClock>;

/// One seeded run driven by the production engine over real SQLite.
pub(super) struct Harness {
    pub(super) temp: TempDir,
    pub(super) pool: RepositoryPool,
    pub(super) run_id: WorkflowRunId,
    pub(super) engine: RetryEngine,
    clock: ManualClock,
    timer: Arc<RecordingTimer>,
    dispatches: DispatchLog,
}

impl Harness {
    /// Seeds a `Pending` run of `graph` without starting it.
    pub(super) fn pending(graph: &str) -> Self {
        let (temp, pool) = bootstrap();
        let run_id = seeded_pending_run(&temp, &pool, graph);
        let clock = ManualClock::default();
        clock.set(40);
        let timer = Arc::new(RecordingTimer::default());
        let dispatches = DispatchLog::default();
        let engine = WorkflowRunEngine::new(
            SqliteWorkflowRunEngineRepository::new(pool.clone()),
            dispatches.clone(),
            SeqGen::default(),
            clock.clone(),
        )
        .with_retry_timer(timer.clone());
        Self {
            temp,
            pool,
            run_id,
            engine,
            clock,
            timer,
            dispatches,
        }
    }

    /// Seeds and starts a run of `graph` at t = 40 ms.
    pub(super) fn start(graph: &str) -> Self {
        let harness = Self::pending(graph);
        harness.engine.start(&harness.run_id).unwrap();
        harness
    }

    pub(super) fn repository(&self) -> SqliteWorkflowRunEngineRepository {
        SqliteWorkflowRunEngineRepository::new(self.pool.clone())
    }

    /// Live rows of one node in creation order.
    pub(super) fn rows_of(&self, node_id: &str) -> Vec<WorkflowNodeRun> {
        self.repository()
            .list_node_runs(&self.run_id)
            .unwrap()
            .into_iter()
            .filter(|row| row.node_id == node_id)
            .collect()
    }

    /// The single live `Running` row of one node.
    pub(super) fn running(&self, node_id: &str) -> WorkflowNodeRun {
        let running: Vec<_> = self
            .rows_of(node_id)
            .into_iter()
            .filter(|row| row.status == WorkflowNodeStatus::Running)
            .collect();
        assert_eq!(running.len(), 1, "exactly one running {node_id} row");
        running.into_iter().next().unwrap()
    }

    /// Moves the shared clock, as the time of the next engine call.
    pub(super) fn set_now(&self, now: i64) {
        self.clock.set(now);
    }

    pub(super) fn fail(&self, node_run_id: &WorkflowNodeRunId, kind: NodeFailureKind, at: i64) {
        self.fail_with(
            node_run_id,
            NodeFailure::new(kind, format!("{kind:?} at {at}")),
            at,
        );
    }

    pub(super) fn fail_with(&self, node_run_id: &WorkflowNodeRunId, failure: NodeFailure, at: i64) {
        self.clock.set(at);
        self.engine
            .fail_node(&self.run_id, node_run_id, failure)
            .unwrap();
    }

    pub(super) fn complete(&self, node_run_id: &WorkflowNodeRunId, output: &str, at: i64) {
        self.clock.set(at);
        self.engine
            .complete_node(
                &self.run_id,
                node_run_id,
                Some(output.to_string()),
                /*structured_output*/ None,
                /*stop_reason*/ None,
                Vec::new(),
            )
            .unwrap();
    }

    /// Delivers one timer wake for `node_run_id` at `at`.
    pub(super) fn wake(&self, node_run_id: &WorkflowNodeRunId, at: i64) {
        self.clock.set(at);
        self.engine.wake_retry(&self.run_id, node_run_id).unwrap();
    }

    pub(super) fn run(&self) -> WorkflowRun {
        SqliteWorkflowRunRepository::new(self.pool.clone())
            .find_run(&self.run_id)
            .unwrap()
            .unwrap()
    }

    pub(super) fn detail(&self) -> WorkflowRunDetail {
        SqliteWorkflowRunRepository::new(self.pool.clone())
            .get_run_detail(&self.run_id)
            .unwrap()
            .unwrap()
    }

    pub(super) fn armed(&self) -> Vec<(String, i64)> {
        self.timer.armed.lock().unwrap().clone()
    }

    pub(super) fn dispatches(&self) -> Vec<Dispatch> {
        self.dispatches.0.lock().unwrap().clone()
    }

    /// Node-run ids dispatched for one node, in dispatch order.
    pub(super) fn dispatched(&self, node_id: &str) -> Vec<String> {
        self.dispatches()
            .into_iter()
            .filter(|dispatch| dispatch.node_id == node_id)
            .map(|dispatch| dispatch.node_run_id)
            .collect()
    }

    /// The persisted waiting marker of one row, if it is waiting.
    pub(super) fn wait_of(row: &WorkflowNodeRun) -> Option<NodeRetryWait> {
        let payload: Value = serde_json::from_str(row.payload.as_deref()?).unwrap();
        payload
            .get(RETRY_WAIT_KEY)
            .map(|wait| serde_json::from_value(wait.clone()).unwrap())
    }

    /// The persisted `payload.error_detail` of one row.
    pub(super) fn error_detail(row: &WorkflowNodeRun) -> Value {
        serde_json::from_str::<Value>(row.payload.as_deref().unwrap_or("{}")).unwrap()
            ["error_detail"]
            .clone()
    }

    /// The prompt the executor assembles for the live `Running` row of `node_id`: same
    /// previous-attempt lookup, same injection switch, same assembler.
    pub(super) fn prompt_for(&self, node_id: &str) -> String {
        let live = self.running(node_id);
        let context = self
            .repository()
            .find_execution_context(&self.run_id)
            .unwrap()
            .unwrap();
        let payload: WorkflowRunPayload =
            serde_json::from_str(context.run.payload.as_deref().unwrap()).unwrap();
        let previous = self
            .repository()
            .find_last_failed_attempt(&self.run_id, node_id, live.iteration)
            .unwrap();
        let injected =
            previous_failure_for_injection(payload.inject_last_failure, previous.as_ref());
        let graph = WorkflowGraph::parse(&context.graph_json).unwrap();
        let node = graph.execution_node(node_id).unwrap().clone();
        assemble_workflow_prompt(WorkflowPromptRequest {
            node: &node,
            graph: None,
            worktree_root: self.temp.path(),
            role_content: None,
            graph_json: &context.graph_json,
            run_input: None,
            node_runs: &[],
            required_skills: &[],
            locale: payload.locale,
            previous_failure: injected.as_ref(),
        })
        .into_iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
    }

    /// Overwrites one key of the run payload, as a run created with other options would carry.
    pub(super) fn set_run_payload_key(&self, key: &str, value: Value) {
        let mut payload: Value =
            serde_json::from_str(self.run().payload.as_deref().unwrap()).unwrap();
        payload[key] = value;
        rusqlite::Connection::open(self.temp.path().join("repository.sqlite3"))
            .unwrap()
            .execute(
                "UPDATE workflow_runs SET payload = ?2 WHERE id = ?1",
                rusqlite::params![self.run_id.as_ref(), payload.to_string()],
            )
            .unwrap();
    }
}

/// An agent node whose `agentConfig` is the base config merged with `extra`.
pub(super) fn agent(id: &str, extra: Value) -> Value {
    let mut config =
        json!({"executor": {"agentCli": "c", "modelId": "m"}, "prompt": format!("do {id}")});
    if let Value::Object(extra) = extra {
        for (key, value) in extra {
            config[key] = value;
        }
    }
    json!({"id": id, "data": {"kind": "agent", "agentConfig": config}})
}

/// A graph document from nodes and `(source, target)` edges.
pub(super) fn graph(nodes: Vec<Value>, edges: &[(&str, &str)]) -> String {
    let edges: Vec<Value> = edges
        .iter()
        .map(|(source, target)| json!({"source": source, "target": target}))
        .collect();
    json!({"nodes": nodes, "edges": edges}).to_string()
}

fn start_node() -> Value {
    json!({"id": "start", "data": {"kind": "start"}})
}

/// `start → a → out`, with `a` configured by `extra`.
pub(super) fn linear(extra: Value) -> String {
    graph(
        vec![
            start_node(),
            agent("a", extra),
            json!({"id": "out", "data": {"kind": "output"}}),
        ],
        &[("start", "a"), ("a", "out")],
    )
}

/// Two independent agents `a` and `b` after Start.
pub(super) fn siblings(a: Value, b: Value) -> String {
    graph(
        vec![start_node(), agent("a", a), agent("b", b)],
        &[("start", "a"), ("start", "b")],
    )
}

pub(super) fn retry(enabled: bool, max_retries: u32, initial_delay_seconds: u32) -> Value {
    json!({"retry": {"enabled": enabled, "maxRetries": max_retries, "initialDelaySeconds": initial_delay_seconds}})
}

/// Test 1: a structured-output failure is retried once after exactly ten seconds; the retry's
/// prompt carries the failure block exactly once, and the first attempt stays readable history.
#[test]
fn structured_output_failure_retries_after_ten_seconds_with_the_failure_injected_once() {
    let h = Harness::start(&linear(json!({})));
    let first = h.running("a");
    assert!(!h.prompt_for("a").contains("Previous attempt"));
    h.fail_with(
        &first.id,
        NodeFailure::new(NodeFailureKind::StructuredOutput, "reply is not JSON")
            .with_output(Some("plain text".to_string()))
            .with_source_chain(vec!["expected value at line 1 column 1".to_string()]),
        1_000,
    );

    let waiting = h.running("a");
    assert_ne!(waiting.id, first.id);
    assert_eq!(waiting.started_at, None);
    assert_eq!(
        Harness::wait_of(&waiting),
        Some(NodeRetryWait {
            attempt: 2,
            max_attempt: 3,
            retry: 1,
            max_retries: 2,
            delay_ms: 10_000,
            scheduled_at: 1_000,
            due_at: 11_000,
            previous_node_run_id: first.id.to_string(),
        })
    );
    assert_eq!(h.armed(), vec![(waiting.id.to_string(), 11_000)]);
    assert_eq!(h.run().status, WorkflowRunStatus::Running);
    assert_eq!(h.dispatched("a"), vec![first.id.to_string()]);

    // An early wake starts nothing and re-arms for the persisted deadline.
    h.wake(&waiting.id, 10_999);
    assert_eq!(h.dispatched("a"), vec![first.id.to_string()]);
    assert_eq!(h.armed().last(), Some(&(waiting.id.to_string(), 11_000)));

    h.wake(&waiting.id, 11_000);
    assert_eq!(
        h.dispatched("a"),
        vec![first.id.to_string(), waiting.id.to_string()]
    );
    let started = h.running("a");
    assert_eq!(
        (&started.id, started.started_at, Harness::wait_of(&started)),
        (&waiting.id, Some(11_000), None)
    );
    assert_eq!(
        NodeAutoRetry::from_payload(started.payload.as_deref()),
        Some(NodeAutoRetry {
            retry: 1,
            max_retries: 2
        })
    );
    let prompt = h.prompt_for("a");
    assert_eq!(prompt.matches("Previous attempt").count(), 1, "{prompt}");
    assert!(prompt.contains("Previous attempt (1) failed"), "{prompt}");
    assert!(prompt.contains("reply is not JSON"), "{prompt}");
    assert!(prompt.contains("plain text"), "{prompt}");

    h.complete(&started.id, "done", 12_000);
    assert_eq!(h.run().status, WorkflowRunStatus::Succeeded);
    let response =
        GetWorkflowRunHandler::new(Arc::new(SqliteWorkflowRunRepository::new(h.pool.clone())))
            .handle(GetWorkflowRunRequest {
                run_id: h.run_id.to_string(),
            })
            .unwrap();
    assert_eq!(
        response.failed_attempts,
        Some(vec![WorkflowNodeFailedAttempt {
            node_run_id: first.id.to_string(),
            node_id: "a".to_string(),
            scope_id: first.scope_id.to_string(),
            iteration: None,
            session_id: None,
            attempt: 1,
            kind: "structured_output".to_string(),
            message: "reply is not JSON".to_string(),
            source_chain: vec!["expected value at line 1 column 1".to_string()],
            recorded_at: 1_000,
            started_at: first.started_at,
            finished_at: Some(1_000),
        }])
    );
}

/// Test 2: three session failures wait 10 s then 20 s, then fail the run on attempt 3, which
/// stays resumable.
#[test]
fn three_session_failures_wait_ten_then_twenty_seconds_and_then_fail_the_run() {
    let h = Harness::start(&linear(json!({})));
    h.fail(&h.running("a").id, NodeFailureKind::Session, 1_000);
    let second = h.running("a");
    assert_eq!(Harness::wait_of(&second).unwrap().due_at, 11_000);
    h.wake(&second.id, 11_000);
    h.fail(&second.id, NodeFailureKind::Session, 12_000);
    let third = h.running("a");
    let wait = Harness::wait_of(&third).unwrap();
    assert_eq!(
        (
            wait.attempt,
            wait.max_attempt,
            wait.retry,
            wait.delay_ms,
            wait.due_at
        ),
        (3, 3, 2, 20_000, 32_000)
    );
    h.wake(&third.id, 32_000);
    h.fail(&third.id, NodeFailureKind::Session, 33_000);

    assert_eq!(
        h.armed(),
        vec![
            (second.id.to_string(), 11_000),
            (third.id.to_string(), 32_000)
        ]
    );
    let run = h.run();
    assert_eq!(run.status, WorkflowRunStatus::Failed);
    let rows = h.rows_of("a");
    assert_eq!(rows.len(), 1);
    let detail = Harness::error_detail(&rows[0]);
    assert_eq!(
        (
            rows[0].status,
            detail["attempt"].clone(),
            detail["kind"].clone(),
            detail["resumable"].clone()
        ),
        (
            WorkflowNodeStatus::Failed,
            json!(3),
            json!("session"),
            json!(true)
        )
    );
    assert_eq!(
        h.detail()
            .failed_attempts
            .iter()
            .map(|row| Harness::error_detail(row)["attempt"].clone())
            .collect::<Vec<_>>(),
        vec![json!(1), json!(2)]
    );
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
}

/// Every failure kind and whether automatic retry covers it.
const KIND_RETRIES: [(NodeFailureKind, bool); 17] = [
    (NodeFailureKind::Session, true),
    (NodeFailureKind::SessionEndedWithoutStopReason, true),
    (NodeFailureKind::SessionBindingRejected, true),
    (NodeFailureKind::StructuredOutput, true),
    (NodeFailureKind::AgentRefusal, true),
    (NodeFailureKind::UnknownStopReason, true),
    (NodeFailureKind::MissingAgentRef, false),
    (NodeFailureKind::WorkflowModelNotFound, false),
    (NodeFailureKind::MissingAgentConfig, false),
    (NodeFailureKind::InvalidRunPayload, false),
    (NodeFailureKind::PromptTemplate, false),
    (NodeFailureKind::MissingSkillMaterialization, false),
    (NodeFailureKind::BaselinePersist, false),
    (NodeFailureKind::Repository, false),
    (NodeFailureKind::InterruptedByRestart, false),
    (NodeFailureKind::MultipleOutputs, false),
    (NodeFailureKind::ConditionEvaluation, false),
];

/// Test 3: exactly the session and agent-answer kinds schedule a retry through the engine.
#[test]
fn only_session_and_agent_answer_failures_schedule_a_retry() {
    let observed: Vec<_> = KIND_RETRIES
        .iter()
        .map(|(kind, _)| {
            let h = Harness::start(&linear(json!({})));
            h.fail(&h.running("a").id, *kind, 1_000);
            let waiting = h
                .rows_of("a")
                .iter()
                .any(|row| Harness::wait_of(row).is_some());
            (
                *kind,
                waiting && h.run().status == WorkflowRunStatus::Running,
            )
        })
        .collect();
    assert_eq!(observed, KIND_RETRIES.to_vec());
}

/// Every recorded failure says whether its kind is retried automatically, also when the node's
/// policy is off: the run view uses it to explain why an agent with retry on failed at once.
#[test]
fn every_failure_records_whether_its_kind_is_retried_automatically() {
    for (kind, retried) in KIND_RETRIES {
        for policy in [json!({}), retry(false, 0, 0)] {
            let h = Harness::start(&linear(policy.clone()));
            let first = h.running("a");
            h.fail(&first.id, kind, 1_000);
            let recorded = h
                .detail()
                .failed_attempts
                .into_iter()
                .chain(h.rows_of("a"))
                .find(|row| row.id == first.id)
                .unwrap();
            assert_eq!(
                Harness::error_detail(&recorded)["auto_retryable"],
                json!(retried),
                "{kind:?} under {policy}"
            );
        }
    }
}

/// A recorded failure promises injection only when a later attempt will really be told about
/// it: an agent-answer failure in a run created with `injectLastFailure` off records `false`
/// (the run view must not promise injection there), and a session failure never records `true`.
/// A payload without the key is an older run, which injects.
#[test]
fn recorded_failures_promise_injection_only_when_the_run_injects() {
    let cases = [
        (NodeFailureKind::StructuredOutput, None, true),
        (NodeFailureKind::StructuredOutput, Some(true), true),
        (NodeFailureKind::StructuredOutput, Some(false), false),
        (NodeFailureKind::Session, None, false),
        (NodeFailureKind::Session, Some(true), false),
        (NodeFailureKind::Session, Some(false), false),
    ];
    for (kind, switch, injects) in cases {
        // Both the retried path and the path that fails the run record the detail.
        for policy in [json!({}), retry(false, 0, 0)] {
            let h = Harness::start(&linear(policy.clone()));
            match switch {
                Some(on) => h.set_run_payload_key("injectLastFailure", json!(on)),
                None => {
                    rusqlite::Connection::open(h.temp.path().join("repository.sqlite3"))
                        .unwrap()
                        .execute(
                            "UPDATE workflow_runs SET payload = json_remove(payload, '$.injectLastFailure') WHERE id = ?1",
                            rusqlite::params![h.run_id.as_ref()],
                        )
                        .unwrap();
                    assert!(!h.run().payload.unwrap().contains("injectLastFailure"));
                }
            }
            let first = h.running("a");
            h.fail(&first.id, kind, 1_000);
            let recorded = h
                .detail()
                .failed_attempts
                .into_iter()
                .chain(h.rows_of("a"))
                .find(|row| row.id == first.id)
                .unwrap();
            assert_eq!(
                Harness::error_detail(&recorded)["injects_previous_failure"],
                json!(injects),
                "{kind:?} with injectLastFailure {switch:?} under {policy}"
            );
        }
    }
}

/// Test 4: a disabled policy, a zero budget, and interactive nodes never retry; a node without
/// `retry` gets the default policy.
#[test]
fn disabled_zero_budget_and_interactive_nodes_never_retry() {
    let cases = [
        ("enabled false", retry(false, 2, 10), None),
        ("maxRetries 0", retry(true, 0, 10), None),
        ("missing retry", json!({}), Some(11_000)),
        ("interactive", json!({"interactive": true}), None),
        (
            "interactive with a policy",
            json!({"interactive": true, "retry": {"enabled": true, "maxRetries": 5, "initialDelaySeconds": 1}}),
            None,
        ),
    ];
    for (name, extra, due_at) in cases {
        let h = Harness::start(&linear(extra));
        h.fail(&h.running("a").id, NodeFailureKind::Session, 1_000);
        let armed: Vec<i64> = h.armed().into_iter().map(|(_, due)| due).collect();
        let expected_status = match due_at {
            Some(_) => WorkflowRunStatus::Running,
            None => WorkflowRunStatus::Failed,
        };
        assert_eq!(
            (name, armed, h.run().status),
            (name, due_at.into_iter().collect(), expected_status)
        );
    }
}

/// Test 5 (engine half): with `initialDelaySeconds = 300` and five retries the waits are
/// 300, 600, 600, 600, 600 seconds, and the sixth failure fails the run.
#[test]
fn backoff_doubles_from_the_initial_delay_and_caps_at_ten_minutes() {
    let h = Harness::start(&linear(retry(true, 5, 300)));
    let mut now = 1_000;
    let mut delays = Vec::new();
    loop {
        h.fail(&h.running("a").id, NodeFailureKind::Session, now);
        let Some(wait) = h.rows_of("a").iter().find_map(Harness::wait_of) else {
            break;
        };
        delays.push(wait.delay_ms);
        assert_eq!(wait.due_at, now + wait.delay_ms);
        now = wait.due_at;
        h.wake(&h.running("a").id, now);
        now += 1;
    }
    assert_eq!(delays, vec![300_000, 600_000, 600_000, 600_000, 600_000]);
    assert_eq!(h.run().status, WorkflowRunStatus::Failed);
    assert_eq!(
        Harness::error_detail(&h.rows_of("a")[0])["attempt"],
        json!(6)
    );
}

/// Test 6: cancelling during the wait cancels the waiting attempt at once, and the timer that
/// fires afterwards dispatches nothing.
#[test]
fn cancel_during_the_wait_cancels_immediately_and_the_late_wake_is_a_no_op() {
    let h = Harness::start(&linear(json!({})));
    h.fail(&h.running("a").id, NodeFailureKind::Session, 1_000);
    let waiting = h.running("a");
    h.clock.set(2_000);
    assert_eq!(
        h.engine.cancel(&h.run_id).unwrap(),
        CancelWorkflowRunResult::Cancelled
    );
    let cancelled = h.rows_of("a").pop().unwrap();
    assert_eq!(
        (
            &cancelled.id,
            cancelled.status,
            cancelled.finished_at,
            Harness::wait_of(&cancelled)
        ),
        (
            &waiting.id,
            WorkflowNodeStatus::Cancelled,
            Some(2_000),
            None
        )
    );

    h.wake(&waiting.id, 11_000);
    assert_eq!(h.dispatched("a").len(), 1);
    assert_eq!(h.rows_of("a").pop().unwrap(), cancelled);
    assert_eq!(h.run().status, WorkflowRunStatus::Cancelled);
}

/// Test 7: a sibling's final failure during the wait fails the run, abandons the waiting
/// attempt so its timer never fires, and resume reruns both nodes.
#[test]
fn a_sibling_final_failure_abandons_the_wait_and_resume_reruns_both_nodes() {
    let h = Harness::start(&siblings(json!({}), json!({})));
    h.fail(&h.running("a").id, NodeFailureKind::Session, 1_000);
    let waiting = h.running("a");
    h.fail(&h.running("b").id, NodeFailureKind::PromptTemplate, 2_000);

    assert_eq!(h.run().status, WorkflowRunStatus::Failed);
    let abandoned = h.rows_of("a").pop().unwrap();
    assert_eq!(
        (
            &abandoned.id,
            abandoned.status,
            abandoned.error.as_deref(),
            Harness::wait_of(&abandoned)
        ),
        (
            &waiting.id,
            WorkflowNodeStatus::Cancelled,
            Some(r#"{"reason":"retry_abandoned"}"#),
            None
        )
    );
    h.wake(&waiting.id, 11_000);
    assert_eq!(h.dispatched("a").len(), 1);

    h.clock.set(20_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    assert_eq!((h.dispatched("a").len(), h.dispatched("b").len()), (2, 2));
    // The resumed attempt starts a fresh budget: it carries no automatic-retry marker.
    assert_eq!(
        NodeAutoRetry::from_payload(h.running("a").payload.as_deref()),
        None
    );
}

/// Test 8: an independent branch keeps running during the wait, the run is not finished, and
/// the waiting node's successor is dispatched only after the retry succeeds.
#[test]
fn independent_branches_progress_while_downstream_waits_for_the_retry() {
    let h = Harness::start(&graph(
        vec![
            start_node(),
            agent("a", json!({})),
            agent("after", json!({})),
            agent("b", json!({})),
        ],
        &[("start", "a"), ("a", "after"), ("start", "b")],
    ));
    h.fail(&h.running("a").id, NodeFailureKind::AgentRefusal, 1_000);
    let waiting = h.running("a");
    h.complete(&h.running("b").id, "b done", 2_000);

    assert_eq!(h.run().status, WorkflowRunStatus::Running);
    assert_eq!(h.rows_of("b")[0].status, WorkflowNodeStatus::Succeeded);
    assert!(h.dispatched("after").is_empty());

    h.wake(&waiting.id, 11_000);
    assert!(h.dispatched("after").is_empty());
    h.complete(&waiting.id, "a done", 12_000);
    assert_eq!(h.dispatched("after").len(), 1);
    h.complete(&h.running("after").id, "after done", 13_000);
    assert_eq!(h.run().status, WorkflowRunStatus::Succeeded);
}

/// Test 10: a manual resume after an exhausted budget starts a fresh budget while attempt
/// numbers keep counting: attempts 4, 5, 6.
#[test]
fn manual_resume_after_exhaustion_resets_the_budget_and_keeps_counting_attempts() {
    let h = Harness::start(&linear(json!({})));
    let mut now = 1_000;
    let mut waits = Vec::new();
    for round in 0..2 {
        loop {
            h.fail(&h.running("a").id, NodeFailureKind::Session, now);
            let Some(wait) = h.rows_of("a").iter().find_map(Harness::wait_of) else {
                break;
            };
            waits.push((wait.attempt, wait.max_attempt, wait.retry, wait.delay_ms));
            now = wait.due_at;
            h.wake(&h.running("a").id, now);
            now += 1;
        }
        assert_eq!(h.run().status, WorkflowRunStatus::Failed);
        if round == 0 {
            h.clock.set(now);
            assert_eq!(
                h.engine.resume_from_failure(&h.run_id).unwrap(),
                ResumeWorkflowRunResult::Resumed
            );
        }
    }
    assert_eq!(
        waits,
        vec![
            (2, 3, 1, 10_000),
            (3, 3, 2, 20_000),
            (5, 6, 1, 10_000),
            (6, 6, 2, 20_000),
        ]
    );
    assert_eq!(
        Harness::error_detail(&h.rows_of("a")[0])["attempt"],
        json!(6)
    );
    assert_eq!(
        h.detail()
            .failed_attempts
            .iter()
            .map(|row| Harness::error_detail(row)["attempt"].clone())
            .collect::<Vec<_>>(),
        vec![json!(1), json!(2), json!(3), json!(4), json!(5)]
    );
}

/// Test 13: with the run's injection switch off the retry prompt has no failure block, and a
/// session-class retry never gets one.
#[test]
fn retries_inject_nothing_when_the_switch_is_off_or_the_failure_was_the_session() {
    let switched_off = Harness::start(&linear(json!({})));
    switched_off.set_run_payload_key("injectLastFailure", json!(false));
    switched_off.fail(
        &switched_off.running("a").id,
        NodeFailureKind::StructuredOutput,
        1_000,
    );
    switched_off.wake(&switched_off.running("a").id, 11_000);
    assert_eq!(switched_off.dispatched("a").len(), 2);
    assert!(!switched_off.prompt_for("a").contains("Previous attempt"));

    let session = Harness::start(&linear(json!({})));
    session.fail(&session.running("a").id, NodeFailureKind::Session, 1_000);
    session.wake(&session.running("a").id, 11_000);
    assert_eq!(session.dispatched("a").len(), 2);
    assert!(!session.prompt_for("a").contains("Previous attempt"));
}

/// Test 14: late and duplicate callbacks for a replaced attempt, and repeated wakes, change
/// nothing; two waits with different deadlines each fire on their own.
#[test]
fn late_callbacks_and_repeated_wakes_are_no_ops_and_concurrent_waits_fire_independently() {
    let h = Harness::start(&siblings(json!({}), retry(true, 1, 30)));
    let first_a = h.running("a");
    h.fail(&first_a.id, NodeFailureKind::Session, 1_000);
    let waiting_a = h.running("a");

    // The replaced attempt's session reports again: failure, then success.
    h.fail(&first_a.id, NodeFailureKind::Session, 1_500);
    h.complete(&first_a.id, "late", 1_600);
    assert_eq!(h.rows_of("a"), vec![h.running("a")]);
    assert_eq!(Harness::wait_of(&h.running("a")).unwrap().due_at, 11_000);
    assert_eq!(h.armed().len(), 1);

    h.fail(&h.running("b").id, NodeFailureKind::Session, 2_000);
    let waiting_b = h.running("b");
    assert_eq!(
        h.armed(),
        vec![
            (waiting_a.id.to_string(), 11_000),
            (waiting_b.id.to_string(), 32_000)
        ]
    );

    h.wake(&waiting_a.id, 11_000);
    h.wake(&waiting_a.id, 11_001);
    h.wake(&waiting_b.id, 11_002);
    assert_eq!((h.dispatched("a").len(), h.dispatched("b").len()), (2, 1));
    assert_eq!(h.armed().last(), Some(&(waiting_b.id.to_string(), 32_000)));
    h.wake(&waiting_b.id, 32_000);
    assert_eq!((h.dispatched("a").len(), h.dispatched("b").len()), (2, 2));

    // A wake for an attempt that already finished is a no-op too.
    h.complete(&waiting_a.id, "a done", 33_000);
    h.wake(&waiting_a.id, 40_000);
    assert_eq!(h.dispatched("a").len(), 2);
    assert_eq!(h.rows_of("a")[0].status, WorkflowNodeStatus::Succeeded);
}
