//! Resume behaviours that the original engine/backend suites did not cover end to end.

use super::last_failure::previous_failure_for_injection;
use super::prompt::{WorkflowPromptRequest, assemble_workflow_prompt, render_previous_failure};
use super::recovery::sweep_one_run;
use super::snapshot_switch::switch_if_requested;
use super::test_fixture::{
    AGENT_GRAPH, ClockAt, RecordingExecutor, SeqGen, bootstrap, started_run_with,
};
use agent_client_protocol_schema::v1::ContentBlock;
use ora_application::{
    CancelWorkflowRunResult, NodeFailure, NodeFailureKind, ResumeWorkflowRunResult, WorkflowGraph,
    WorkflowRunEngine, WorkflowRunEngineRepository, WorkflowRunPayload, WorkflowRunRepository,
};
use ora_contracts::WorkflowRunLocale;
use ora_db::{SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository};
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use rusqlite::params;
use std::collections::BTreeMap;
use std::path::Path;
use tempfile::TempDir;

const LINEAR_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"c","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"c"}}},
    {"id":"output","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"a","target":"c"},
    {"source":"c","target":"output"}
]}"#;

const THREE_SIBLING_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"b","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"b"}}},
    {"id":"c","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"c"}}},
    {"id":"d","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"d"}}},
    {"id":"output","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"start","target":"b"},
    {"source":"start","target":"c"},
    {"source":"a","target":"d"},
    {"source":"b","target":"d"},
    {"source":"c","target":"d"},
    {"source":"d","target":"output"}
]}"#;

const V1_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"c","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"old-c"}}},
    {"id":"output","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"a","target":"c"},
    {"source":"c","target":"output"}
]}"#;

const V2_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"c","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"fixed-c"}}},
    {"id":"output","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"a","target":"c"},
    {"source":"c","target":"output"}
]}"#;

type FixtureEngine = WorkflowRunEngine<SqliteWorkflowRunEngineRepository, SeqGen, ClockAt>;

fn live_nodes(pool: &ora_db::RepositoryPool, run_id: &WorkflowRunId) -> Vec<WorkflowNodeRun> {
    SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(run_id)
        .unwrap()
}

fn live<'a>(nodes: &'a [WorkflowNodeRun], node_id: &str) -> &'a WorkflowNodeRun {
    nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .unwrap_or_else(|| panic!("missing live node {node_id}"))
}

fn dispatch_counts(executor: &RecordingExecutor) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for node_id in executor.dispatches.lock().expect("dispatch log").iter() {
        *counts.entry(node_id.clone()).or_insert(0) += 1;
    }
    counts
}

fn find_run(pool: &ora_db::RepositoryPool, run_id: &WorkflowRunId) -> ora_domain::WorkflowRun {
    SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(run_id)
        .unwrap()
        .unwrap()
}

fn complete(
    engine: &FixtureEngine,
    run_id: &WorkflowRunId,
    node_run_id: &ora_domain::WorkflowNodeRunId,
) {
    engine
        .complete_node(
            run_id,
            node_run_id,
            Some("ok".to_string()),
            /*structured_output*/ None,
            /*stop_reason*/ None,
            Vec::new(),
        )
        .unwrap();
}

fn repository(pool: &ora_db::RepositoryPool) -> SqliteWorkflowRunEngineRepository {
    SqliteWorkflowRunEngineRepository::new(pool.clone())
}

fn parse_payload(raw: Option<&str>) -> serde_json::Value {
    serde_json::from_str(raw.unwrap_or("{}")).unwrap()
}

fn run_payload(pool: &ora_db::RepositoryPool, run_id: &WorkflowRunId) -> WorkflowRunPayload {
    serde_json::from_str(
        find_run(pool, run_id)
            .payload
            .as_deref()
            .expect("run payload"),
    )
    .unwrap()
}

fn set_run_payload_json(temp: &TempDir, run_id: &str, payload: serde_json::Value) {
    let connection = rusqlite::Connection::open(temp.path().join("repository.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE workflow_runs SET payload = ?2 WHERE id = ?1",
            params![run_id, payload.to_string()],
        )
        .unwrap();
}

fn prompt_text(blocks: Vec<ContentBlock>) -> String {
    blocks
        .into_iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assembled_prompt(
    graph: &str,
    node_id: &str,
    previous: Option<&ora_domain::WorkflowNodeRun>,
    inject: bool,
    worktree: &Path,
) -> String {
    let parsed = WorkflowGraph::parse(graph).unwrap();
    let node = parsed.node(node_id).expect("graph node").clone();
    let injected = previous_failure_for_injection(inject, previous);
    prompt_text(assemble_workflow_prompt(WorkflowPromptRequest {
        node: &node,
        graph: None,
        worktree_root: worktree,
        role_content: None,
        graph_json: graph,
        run_input: None,
        node_runs: &[],
        required_skills: &[],
        locale: WorkflowRunLocale::EnUs,
        previous_failure: injected.as_ref(),
    }))
}

/// Apply the executor's injection-recording decision onto the live node row.
fn record_injection_like_executor(
    pool: &ora_db::RepositoryPool,
    run_id: &WorkflowRunId,
    node_id: &str,
) {
    let payload = run_payload(pool, run_id);
    let live_row = live(&live_nodes(pool, run_id), node_id).clone();
    let previous = repository(pool)
        .find_last_failed_attempt(run_id, node_id, live_row.iteration)
        .unwrap();
    let injected = previous_failure_for_injection(payload.inject_last_failure, previous.as_ref());
    if let Some(previous) = injected.as_ref() {
        repository(pool)
            .record_node_injected_failure(
                &live_row.id,
                &render_previous_failure(previous, payload.locale),
            )
            .unwrap();
    }
}

/// B2: cancel mid-run, then resume reruns the cancelled node and later its descendant.
#[test]
fn resume_after_cancel_reruns_cancelled_nodes_and_descendants() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, LINEAR_GRAPH, executor.clone());
        let nodes = live_nodes(&pool, &run_id);
        complete(&engine, &run_id, &live(&nodes, "a").id);
        let after_a = live_nodes(&pool, &run_id);
        let a_id = live(&after_a, "a").id.clone();
        let c_old = live(&after_a, "c").id.clone();
        assert_eq!(
            engine.cancel(&run_id).unwrap(),
            CancelWorkflowRunResult::Cancelled
        );
        assert_eq!(
            find_run(&pool, &run_id).status,
            WorkflowRunStatus::Cancelled
        );
        assert_eq!(
            live(&live_nodes(&pool, &run_id), "c").status,
            WorkflowNodeStatus::Cancelled
        );

        assert_eq!(
            engine.resume_from_failure(&run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        let after_resume = live_nodes(&pool, &run_id);
        assert_eq!(live(&after_resume, "a").id, a_id);
        assert_eq!(
            live(&after_resume, "a").status,
            WorkflowNodeStatus::Succeeded
        );
        let c_new = live(&after_resume, "c");
        assert_ne!(c_new.id, c_old);
        assert_eq!(c_new.status, WorkflowNodeStatus::Running);
        assert!(after_resume.iter().all(|node| node.node_id != "output"));
        assert_eq!(dispatch_counts(&executor).get("a"), Some(&1));
        assert_eq!(dispatch_counts(&executor).get("c"), Some(&2));

        complete(&engine, &run_id, &c_new.id);
        let finished = live_nodes(&pool, &run_id);
        assert_eq!(
            live(&finished, "output").status,
            WorkflowNodeStatus::Succeeded
        );
        assert_eq!(
            find_run(&pool, &run_id).status,
            WorkflowRunStatus::Succeeded
        );
    });
}

/// B3: the third attempt injects attempt 2's failure, not attempt 1's.
#[test]
fn third_resume_attempt_injects_the_second_failure() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, LINEAR_GRAPH, executor);
        let nodes = live_nodes(&pool, &run_id);
        complete(&engine, &run_id, &live(&nodes, "a").id);
        let c1 = live(&live_nodes(&pool, &run_id), "c").id.clone();
        engine
            .fail_node(
                &run_id,
                &c1,
                NodeFailure::new(NodeFailureKind::StructuredOutput, "first-bad-json"),
            )
            .unwrap();
        engine.resume_from_failure(&run_id).unwrap();
        let c2 = live(&live_nodes(&pool, &run_id), "c").id.clone();
        engine
            .fail_node(
                &run_id,
                &c2,
                NodeFailure::new(NodeFailureKind::StructuredOutput, "second-bad-json"),
            )
            .unwrap();
        engine.resume_from_failure(&run_id).unwrap();
        let after = live_nodes(&pool, &run_id);
        let c3 = live(&after, "c");
        assert_eq!(
            parse_payload(c3.payload.as_deref())
                .get("injected_failure_context")
                .is_some(),
            false
        );
        record_injection_like_executor(&pool, &run_id, "c");
        let injected = parse_payload(live(&live_nodes(&pool, &run_id), "c").payload.as_deref());
        let block = injected["injected_failure_context"].as_str().unwrap();
        assert!(block.contains("Previous attempt (2) failed"), "{block}");
        assert!(block.contains("second-bad-json"), "{block}");
        assert!(!block.contains("first-bad-json"), "{block}");
        let previous = repository(&pool)
            .find_last_failed_attempt(&run_id, "c", None)
            .unwrap()
            .unwrap();
        assert_eq!(
            parse_payload(previous.payload.as_deref())["error_detail"]["attempt"],
            2
        );
        let prompt = assembled_prompt(
            LINEAR_GRAPH,
            "c",
            Some(&previous),
            /*inject*/ true,
            temp.path(),
        );
        assert!(prompt.contains("Previous attempt (2) failed"));
        assert!(prompt.contains("second-bad-json"));
        assert!(!prompt.contains("first-bad-json"));
    });
}

/// B4: `inject_last_failure = false` writes no prompt block and no payload key; omitted field is on.
#[test]
fn inject_last_failure_false_skips_the_prompt_block_and_payload_key() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, LINEAR_GRAPH, executor);
        let mut payload = serde_json::to_value(run_payload(&pool, &run_id)).unwrap();
        payload["injectLastFailure"] = serde_json::json!(false);
        set_run_payload_json(&temp, run_id.as_ref(), payload);
        let nodes = live_nodes(&pool, &run_id);
        complete(&engine, &run_id, &live(&nodes, "a").id);
        let c_old = live(&live_nodes(&pool, &run_id), "c").id.clone();
        engine
            .fail_node(
                &run_id,
                &c_old,
                NodeFailure::new(NodeFailureKind::StructuredOutput, "bad json"),
            )
            .unwrap();
        engine.resume_from_failure(&run_id).unwrap();
        record_injection_like_executor(&pool, &run_id, "c");
        let live_c = live(&live_nodes(&pool, &run_id), "c").clone();
        let parsed = parse_payload(live_c.payload.as_deref());
        assert_eq!(parsed.get("injected_failure_context"), None);
        let previous = repository(&pool)
            .find_last_failed_attempt(&run_id, "c", None)
            .unwrap();
        let prompt = assembled_prompt(
            LINEAR_GRAPH,
            "c",
            previous.as_ref(),
            /*inject*/ false,
            temp.path(),
        );
        assert!(!prompt.contains("Previous attempt"));
        assert!(!prompt.contains("上一次尝试"));
    });
}

/// Older payloads without `inject_last_failure` deserialize as on.
#[test]
fn omitted_inject_last_failure_field_behaves_as_true() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, LINEAR_GRAPH, executor);
        let mut payload = serde_json::to_value(run_payload(&pool, &run_id)).unwrap();
        payload.as_object_mut().unwrap().remove("injectLastFailure");
        set_run_payload_json(&temp, run_id.as_ref(), payload.clone());
        let decoded: WorkflowRunPayload = serde_json::from_value(payload).unwrap();
        assert_eq!(decoded.inject_last_failure, true);
        let nodes = live_nodes(&pool, &run_id);
        complete(&engine, &run_id, &live(&nodes, "a").id);
        engine
            .fail_node(
                &run_id,
                &live(&live_nodes(&pool, &run_id), "c").id,
                NodeFailure::new(NodeFailureKind::StructuredOutput, "bad json"),
            )
            .unwrap();
        engine.resume_from_failure(&run_id).unwrap();
        record_injection_like_executor(&pool, &run_id, "c");
        let block = parse_payload(live(&live_nodes(&pool, &run_id), "c").payload.as_deref());
        assert!(
            block["injected_failure_context"]
                .as_str()
                .is_some_and(|text| text.contains("Previous attempt (1) failed"))
        );
    });
}

/// B5: a `prompt_template` failure is not injected after a compatible snapshot switch.
#[test]
fn prompt_template_failure_is_not_injected_after_snapshot_switch() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, V1_GRAPH, executor);
        let nodes = live_nodes(&pool, &run_id);
        complete(&engine, &run_id, &live(&nodes, "a").id);
        engine
            .fail_node(
                &run_id,
                &live(&live_nodes(&pool, &run_id), "c").id,
                NodeFailure::new(NodeFailureKind::PromptTemplate, "unresolved variable"),
            )
            .unwrap();
        snapshot_switch_tests_publish(&pool, "snapshot-2", "v2", V2_GRAPH, 50);
        let skills_root = temp.path().join("skills");
        std::fs::create_dir_all(&skills_root).unwrap();
        switch_if_requested(
            &pool,
            &skills_root,
            &temp.path().join("fixture-project"),
            &run_id,
            Some("snapshot-2"),
            90,
        )
        .unwrap();
        engine.resume_from_failure(&run_id).unwrap();
        let previous = repository(&pool)
            .find_last_failed_attempt(&run_id, "c", None)
            .unwrap();
        assert_eq!(
            parse_payload(previous.as_ref().unwrap().payload.as_deref())["error_detail"]["kind"],
            "prompt_template"
        );
        assert!(previous_failure_for_injection(true, previous.as_ref()).is_none());
        record_injection_like_executor(&pool, &run_id, "c");
        let parsed = parse_payload(live(&live_nodes(&pool, &run_id), "c").payload.as_deref());
        assert_eq!(parsed.get("injected_failure_context"), None);
        let prompt = assembled_prompt(
            V2_GRAPH,
            "c",
            previous.as_ref(),
            /*inject*/ true,
            temp.path(),
        );
        assert!(!prompt.contains("Previous attempt"));
        assert!(!prompt.contains("unresolved variable"));
    });
}

/// B6: boot sweep marks a running outer node `interrupted_by_restart`, then resume is attempt 2.
#[test]
fn boot_sweep_interrupted_node_resumes_as_attempt_two() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, AGENT_GRAPH, executor.clone());
        let agent_old = live(&live_nodes(&pool, &run_id), "agent").id.clone();
        sweep_one_run(&repository(&pool), &run_id, 80).unwrap();
        let crashed = live(&live_nodes(&pool, &run_id), "agent").clone();
        assert_eq!(crashed.status, WorkflowNodeStatus::Failed);
        let detail = parse_payload(crashed.payload.as_deref())["error_detail"].clone();
        assert_eq!(detail["kind"], "interrupted_by_restart");
        assert_eq!(detail["resumable"], true);
        assert_eq!(detail["injects_previous_failure"], false);
        assert_eq!(detail["attempt"], 1);
        assert_eq!(find_run(&pool, &run_id).status, WorkflowRunStatus::Failed);
        assert_eq!(
            engine.resume_from_failure(&run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        let after_sweep = live_nodes(&pool, &run_id);
        let agent_new = live(&after_sweep, "agent");
        assert_ne!(agent_new.id, agent_old);
        assert_eq!(agent_new.status, WorkflowNodeStatus::Running);
        engine
            .fail_node(
                &run_id,
                &agent_new.id,
                NodeFailure::new(NodeFailureKind::Session, "again"),
            )
            .unwrap();
        let attempt = parse_payload(
            live(&live_nodes(&pool, &run_id), "agent")
                .payload
                .as_deref(),
        );
        assert_eq!(attempt["error_detail"]["attempt"], 2);
        assert_eq!(dispatch_counts(&executor).get("agent"), Some(&2));
    });
}

/// B11: one sibling fails, two in flight finish on their own merits; resume reruns both failures.
#[test]
fn three_siblings_late_outcomes_are_recorded_and_resume_keeps_the_success() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) =
            started_run_with(&temp, &pool, THREE_SIBLING_GRAPH, executor.clone());
        let nodes = live_nodes(&pool, &run_id);
        let a = live(&nodes, "a").clone();
        let b = live(&nodes, "b").clone();
        let c = live(&nodes, "c").clone();
        engine
            .fail_node(
                &run_id,
                &a.id,
                NodeFailure::new(NodeFailureKind::Session, "a failed"),
            )
            .unwrap();
        complete(&engine, &run_id, &b.id);
        engine
            .fail_node(
                &run_id,
                &c.id,
                NodeFailure::new(NodeFailureKind::Session, "c failed later"),
            )
            .unwrap();
        let after = live_nodes(&pool, &run_id);
        assert_eq!(live(&after, "a").status, WorkflowNodeStatus::Failed);
        assert_eq!(live(&after, "b").status, WorkflowNodeStatus::Succeeded);
        assert_eq!(live(&after, "c").status, WorkflowNodeStatus::Failed);
        assert!(after.iter().all(|node| node.node_id != "d"));
        assert_eq!(find_run(&pool, &run_id).status, WorkflowRunStatus::Failed);
        assert_eq!(dispatch_counts(&executor).get("d"), None);

        assert_eq!(
            engine.resume_from_failure(&run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        let resumed = live_nodes(&pool, &run_id);
        assert_eq!(live(&resumed, "b").id, b.id);
        assert_eq!(live(&resumed, "b").status, WorkflowNodeStatus::Succeeded);
        assert_ne!(live(&resumed, "a").id, a.id);
        assert_ne!(live(&resumed, "c").id, c.id);
        assert_eq!(live(&resumed, "a").status, WorkflowNodeStatus::Running);
        assert_eq!(live(&resumed, "c").status, WorkflowNodeStatus::Running);
        assert!(resumed.iter().all(|node| node.node_id != "d"));
        assert_eq!(dispatch_counts(&executor).get("a"), Some(&2));
        assert_eq!(dispatch_counts(&executor).get("b"), Some(&1));
        assert_eq!(dispatch_counts(&executor).get("c"), Some(&2));
        assert_eq!(dispatch_counts(&executor).get("d"), None);
    });
}

/// Publishes a new snapshot onto the fixture workflow so resume-gap tests can switch versions.
fn snapshot_switch_tests_publish(
    pool: &ora_db::RepositoryPool,
    snapshot_id: &str,
    version: &str,
    graph: &str,
    created_at: i64,
) {
    use ora_application::{PublishSnapshotResult, UpdateDraftResult, WorkflowRepository};
    use ora_db::SqliteWorkflowRepository;
    use ora_domain::{WorkflowId, WorkflowSnapshotId};
    let workflow_repo = SqliteWorkflowRepository::new(pool.clone());
    let workflow_id = WorkflowId::new("workflow-1");
    match workflow_repo
        .update_draft(&workflow_id, graph.to_string(), created_at)
        .unwrap()
    {
        UpdateDraftResult::Updated(_) => {}
        other => panic!("expected updated draft, got {other:?}"),
    }
    match workflow_repo
        .publish_snapshot(
            &workflow_id,
            WorkflowSnapshotId::new(snapshot_id),
            version.to_string(),
            created_at,
        )
        .unwrap()
    {
        PublishSnapshotResult::Published(_) => {}
        other => panic!("expected published snapshot, got {other:?}"),
    }
}
