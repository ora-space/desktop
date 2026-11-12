//! Completion gate coverage: error precedence, the legacy materialize path and provisional repair.
//!
//! Every case drives `prepare_completion` against real SQLite rows and a real history file, so the
//! assertions describe the contract a client observes rather than a helper's return value.

use super::super::completion_snapshot::{
    AgentSignal, CompletionBlocker, CompletionBlockerKind, CompletionConfirmation, CompletionMeta,
    CompletionPolicy, CompletionSnapshot, ConfirmationBy, ParkKind, StopCause, TurnKey, TurnOrigin,
    completion_payload, completion_payload_with_meta, parse_completion_snapshot,
};
use super::completion::{
    CompletionApproval, merge_completion_patch, prepare_completion, signal_fingerprint,
};
use crate::workflow::run::test_fixture::{
    INTERACTIVE_AGENT_GRAPH, bind_and_park, bootstrap, end_fixture_turn, run_test, started_run,
    write_open_turn, write_settled_turn,
};
use ora_application::{WorkflowRunEngineRepository, WorkflowRunRepository};
use ora_contracts::{
    NodeCompletionRequester, PublicError, WorkflowNodeCompletionBlocker,
    WorkflowNodeCompletionBlockerKind, WorkflowNodeTurnKey,
};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository};
use ora_domain::{WorkflowNodeRunId, WorkflowRunId};
use ora_utils::hash::sha256_hex;
use pretty_assertions::assert_eq;
use std::path::Path;

/// A stopped turn that has already parked reports the blocker the user must resolve.
fn stopped_snapshot() -> CompletionSnapshot {
    CompletionSnapshot {
        user_seq: Some(0),
        turn_key: Some(TurnKey {
            user_seq: 0,
            ended_seq: 2,
        }),
        stop_reason: Some("cancelled".to_string()),
        turn_origin: Some(TurnOrigin::Human),
        stop_cause: Some(StopCause::UserStopped),
        park_kind: Some(ParkKind::AwaitingUser),
        blockers: vec![CompletionBlocker {
            kind: CompletionBlockerKind::TurnStopped,
            params: serde_json::Value::Null,
        }],
        warnings: Vec::new(),
        output_digest: Some(format!("sha256:{}", sha256_hex(b"partial"))),
        provisional: false,
    }
}

/// Seeds one node's completion snapshot exactly as a park would have written it.
fn store_snapshot(
    pool: &RepositoryPool,
    node_run_id: &WorkflowNodeRunId,
    snapshot: &CompletionSnapshot,
) {
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    assert!(
        repository
            .repair_node_payload(node_run_id, &completion_payload(snapshot), 60)
            .expect("seed completion snapshot"),
        "the node must still be awaiting input when the snapshot is seeded"
    );
}

/// Seeds one snapshot together with the completion policy and signal meta at the payload root.
fn store_snapshot_with_meta(
    pool: &RepositoryPool,
    node_run_id: &WorkflowNodeRunId,
    snapshot: &CompletionSnapshot,
    meta: &CompletionMeta,
) {
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    assert!(
        repository
            .repair_node_payload(
                node_run_id,
                &completion_payload_with_meta(snapshot, meta),
                60,
            )
            .expect("seed completion snapshot with meta"),
        "the node must still be awaiting input when the snapshot is seeded"
    );
}

/// Reads the persisted completion snapshot of the named node.
fn stored_snapshot(pool: &RepositoryPool, run_id: &WorkflowRunId) -> Option<CompletionSnapshot> {
    SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(run_id)
        .expect("node runs")
        .into_iter()
        .find(|node| node.node_id == "agent")
        .and_then(|node| parse_completion_snapshot(node.payload.as_deref()))
}

/// A single interactive agent node whose structured contract rejects anything but `{name}`.
const INTERACTIVE_STRUCTURED_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"agent","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do","interactive":true,"outputContract":{"type":"structured","textExposure":"includeFinalText","schema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}}}}}
],"edges":[{"source":"start","target":"agent"}]}"#;

/// Builds one parked interactive run on `graph` whose session history root is the caller's
/// directory.
fn parked_run_on(
    temp: &tempfile::TempDir,
    pool: &RepositoryPool,
    graph: &str,
) -> (WorkflowRunId, WorkflowNodeRunId, std::path::PathBuf) {
    let (run_id, node_runs) = started_run(temp, pool, graph);
    let agent = node_runs
        .iter()
        .find(|node| node.node_id == "agent")
        .expect("agent node");
    let (_session_id, node_run_id) = bind_and_park(pool, agent);
    (run_id, node_run_id, temp.path().join("sessions"))
}

/// Builds one parked interactive run whose session history root is the caller's directory.
fn parked_run(
    temp: &tempfile::TempDir,
    pool: &RepositoryPool,
) -> (WorkflowRunId, WorkflowNodeRunId, std::path::PathBuf) {
    parked_run_on(temp, pool, INTERACTIVE_AGENT_GRAPH)
}

/// The turn identity is validated before blockers, so a stale completion names its conflict.
#[test]
fn a_changed_turn_wins_over_the_blockers_gate() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "partial");
        write_settled_turn(&sessions_root, "session-1", "carry on", "final");
        store_snapshot(&pool, &node_run_id, &stopped_snapshot());

        let error = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::PLAIN,
        )
        .expect_err("a completion against a superseded turn is rejected");

        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeTurnChanged(ora_contracts::WorkflowNodeTurnChangedParams {
                node_id: "agent".to_string(),
                expected: Some(WorkflowNodeTurnKey {
                    user_seq: 0,
                    ended_seq: 2,
                }),
                actual: Some(WorkflowNodeTurnKey {
                    user_seq: 3,
                    ended_seq: 5,
                }),
            })
        );
    });
}

/// A rewritten final message under the same turn is reported as an output conflict.
#[test]
fn changed_output_text_reports_the_two_digests() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "rewritten");
        let mut stored = stopped_snapshot();
        stored.blockers = Vec::new();
        store_snapshot(&pool, &node_run_id, &stored);

        let error = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::PLAIN,
        )
        .expect_err("a completion against rewritten output is rejected");

        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeOutputChanged(
                ora_contracts::WorkflowNodeOutputChangedParams {
                    node_id: "agent".to_string(),
                    expected: stored.output_digest.clone(),
                    actual: Some(format!("sha256:{}", sha256_hex(b"rewritten"))),
                }
            )
        );
    });
}

/// A parked stopped turn returns its blockers so the client can explain why it cannot advance.
#[test]
fn blocked_turn_returns_every_blocker_with_its_detail() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "partial");
        let mut stored = stopped_snapshot();
        stored.blockers.push(CompletionBlocker {
            kind: CompletionBlockerKind::StructuredInvalid,
            params: serde_json::json!({ "message": "invalid response" }),
        });
        store_snapshot(&pool, &node_run_id, &stored);

        let error = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::PLAIN,
        )
        .expect_err("a blocked turn cannot be completed without force");

        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeIncomplete(ora_contracts::WorkflowNodeIncompleteParams {
                node_id: "agent".to_string(),
                blockers: vec![
                    WorkflowNodeCompletionBlocker {
                        kind: WorkflowNodeCompletionBlockerKind::TurnStopped,
                        message: None,
                    },
                    WorkflowNodeCompletionBlocker {
                        kind: WorkflowNodeCompletionBlockerKind::StructuredInvalid,
                        message: Some("invalid response".to_string()),
                    },
                ],
            })
        );
    });
}

/// The node state is checked before any snapshot comparison, so a terminal node never reports a
/// conflict or a blocker it can no longer act on.
#[test]
fn a_node_that_left_the_awaiting_state_is_rejected_before_its_snapshot() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "partial");
        store_snapshot(&pool, &node_run_id, &stopped_snapshot());
        SqliteWorkflowRunEngineRepository::new(pool.clone())
            .transition_node_run_status(
                &node_run_id,
                ora_domain::WorkflowNodeStatus::Pending,
                ora_domain::WorkflowNodeStatus::Cancelled,
                70,
            )
            .expect("the node leaves the awaiting state");

        let error = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::FORCE,
        )
        .expect_err("a terminal node cannot be completed, even with force");

        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeNotAwaitingInput(ora_contracts::EmptyErrorParams {})
        );
    });
}

/// A node parked before snapshots existed is materialized and then gated like any other node.
#[test]
fn a_legacy_node_is_materialized_and_blocked_by_an_unsettled_turn() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, _node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_open_turn(&sessions_root, "session-1", "go", "still going");
        assert_eq!(stored_snapshot(&pool, &run_id), None, "no snapshot yet");

        let error = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::PLAIN,
        )
        .expect_err("an unsettled turn cannot be completed");
        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeIncomplete(ora_contracts::WorkflowNodeIncompleteParams {
                node_id: "agent".to_string(),
                blockers: vec![WorkflowNodeCompletionBlocker {
                    kind: WorkflowNodeCompletionBlockerKind::TurnUnknown,
                    message: None,
                }],
            })
        );
        // The materialized snapshot keeps the identity a later provisional repair needs.
        assert_eq!(
            stored_snapshot(&pool, &run_id),
            Some(CompletionSnapshot {
                user_seq: Some(0),
                turn_key: None,
                stop_reason: None,
                turn_origin: Some(TurnOrigin::Human),
                stop_cause: None,
                park_kind: Some(ParkKind::Abnormal),
                blockers: vec![CompletionBlocker {
                    kind: CompletionBlockerKind::TurnUnknown,
                    params: serde_json::Value::Null,
                }],
                warnings: Vec::new(),
                output_digest: None,
                provisional: false,
            })
        );
    });
}

/// A settled legacy node completes with its own turn window instead of the pre-snapshot fallback.
#[test]
fn a_legacy_node_with_a_settled_turn_completes_on_its_turn_window() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, _node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(
            &sessions_root,
            "session-1",
            "draft the plan",
            "here is the plan",
        );

        let prepared = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::PLAIN,
        )
        .expect("ready");

        assert_eq!(prepared.output, Some("here is the plan".to_string()));
        assert_eq!(prepared.audit, None);
        assert_eq!(
            stored_snapshot(&pool, &run_id).and_then(|snapshot| snapshot.turn_key),
            Some(TurnKey {
                user_seq: 0,
                ended_seq: 2,
            })
        );
    });
}

/// A provisional snapshot is repaired once the turn settles for the same user message.
#[test]
fn a_provisional_snapshot_is_repaired_for_the_same_user_message() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_open_turn(&sessions_root, "session-1", "go", "done");
        store_snapshot(
            &pool,
            &node_run_id,
            &CompletionSnapshot {
                user_seq: Some(0),
                turn_key: None,
                stop_reason: None,
                turn_origin: Some(TurnOrigin::Human),
                stop_cause: None,
                park_kind: Some(ParkKind::AwaitingUser),
                blockers: vec![CompletionBlocker {
                    kind: CompletionBlockerKind::TurnUnknown,
                    params: serde_json::Value::Null,
                }],
                warnings: vec!["turn_not_settled".to_string()],
                output_digest: None,
                provisional: true,
            },
        );
        end_fixture_turn(
            &sessions_root,
            "session-1",
            agent_client_protocol_schema::v1::StopReason::EndTurn,
        );

        let prepared = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::PLAIN,
        )
        .expect("repaired");

        assert_eq!(prepared.output, Some("done".to_string()));
        let repaired = stored_snapshot(&pool, &run_id).expect("repaired snapshot");
        assert_eq!(
            repaired.turn_key,
            Some(TurnKey {
                user_seq: 0,
                ended_seq: 2,
            })
        );
        assert!(!repaired.provisional);
    });
}

/// A snapshot written before `turnOrigin` existed is repaired as a human turn, not blocked.
///
/// The field is absent on snapshots from older versions. Defaulting it to the workflow's own
/// opening turn would add `turn_unconfirmed` to a turn the user already answered, so the upgrade
/// path must read such a snapshot as human and complete it normally (ADR D7).
#[test]
fn legacy_snapshot_without_turn_origin_defaults_to_human() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "done");
        // Serialize the pre-field park shape: the older version wrote no `turnOrigin`, and the
        // classification fields added with it are absent too.
        let mut legacy = completion_payload(&CompletionSnapshot {
            user_seq: Some(0),
            turn_key: None,
            stop_reason: None,
            turn_origin: Some(TurnOrigin::Human),
            stop_cause: None,
            park_kind: None,
            blockers: vec![CompletionBlocker {
                kind: CompletionBlockerKind::TurnUnknown,
                params: serde_json::Value::Null,
            }],
            warnings: vec!["turn_not_settled".to_string()],
            output_digest: None,
            provisional: true,
        });
        legacy["completion"]
            .as_object_mut()
            .expect("completion snapshot object")
            .remove("turnOrigin");
        assert!(
            SqliteWorkflowRunEngineRepository::new(pool.clone())
                .repair_node_payload(&node_run_id, &legacy, 60)
                .expect("seed the pre-turnOrigin snapshot")
        );
        assert_eq!(
            stored_snapshot(&pool, &run_id),
            Some(CompletionSnapshot {
                user_seq: Some(0),
                turn_key: None,
                stop_reason: None,
                turn_origin: None,
                stop_cause: None,
                park_kind: None,
                blockers: vec![CompletionBlocker {
                    kind: CompletionBlockerKind::TurnUnknown,
                    params: serde_json::Value::Null,
                }],
                warnings: vec!["turn_not_settled".to_string()],
                output_digest: None,
                provisional: true,
            }),
            "the seeded snapshot is the pre-field shape"
        );

        let prepared = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::PLAIN,
        )
        .expect("a pre-turnOrigin snapshot completes as a human turn");

        assert_eq!(prepared.output, Some("done".to_string()));
        assert_eq!(
            stored_snapshot(&pool, &run_id),
            Some(CompletionSnapshot {
                user_seq: Some(0),
                turn_key: Some(TurnKey {
                    user_seq: 0,
                    ended_seq: 2,
                }),
                stop_reason: Some("end_turn".to_string()),
                turn_origin: Some(TurnOrigin::Human),
                stop_cause: None,
                park_kind: None,
                blockers: Vec::new(),
                warnings: Vec::new(),
                output_digest: Some(format!("sha256:{}", sha256_hex(b"done"))),
                provisional: false,
            }),
            "the repair defaults the missing origin to a human turn"
        );
    });
}

/// A newer user message never stands in for the turn a provisional snapshot described.
#[test]
fn a_provisional_snapshot_of_another_turn_is_left_alone() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "first", "one");
        write_settled_turn(&sessions_root, "session-1", "second", "two");
        let provisional = CompletionSnapshot {
            user_seq: Some(0),
            turn_key: None,
            stop_reason: None,
            turn_origin: Some(TurnOrigin::Human),
            stop_cause: None,
            park_kind: Some(ParkKind::AwaitingUser),
            blockers: vec![CompletionBlocker {
                kind: CompletionBlockerKind::TurnUnknown,
                params: serde_json::Value::Null,
            }],
            warnings: vec!["turn_not_settled".to_string()],
            output_digest: None,
            provisional: true,
        };
        store_snapshot(&pool, &node_run_id, &provisional);

        let error = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::PLAIN,
        )
        .expect_err("another turn cannot complete this one");
        assert!(matches!(
            error.public_error(),
            PublicError::WorkflowNodeIncomplete(_)
        ));
        assert_eq!(
            stored_snapshot(&pool, &run_id),
            Some(provisional),
            "a mismatched user message must not be rewritten"
        );
    });
}

/// A forced completion without a settled turn takes the last reply and records why.
#[test]
fn forcing_an_unsettled_turn_records_the_last_assistant_message() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, _node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_open_turn(&sessions_root, "session-1", "go", "draft only");

        let prepared = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::FORCE,
        )
        .expect("forced");

        assert_eq!(prepared.output, Some("draft only".to_string()));
        assert_eq!(
            prepared.audit,
            Some(serde_json::json!({
                "completion_forced": {
                    "by": "human",
                    "blockers": [{ "kind": "turn_unknown" }],
                    "stop_cause": null,
                    "without_turn": true,
                    "output_source": "last_assistant_message",
                }
            }))
        );
    });
}

/// A forced completion of a stopped but settled turn records the turn window as its source.
#[test]
fn forcing_a_stopped_turn_records_the_turn_window() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "partial");
        store_snapshot(&pool, &node_run_id, &stopped_snapshot());

        let prepared = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::FORCE,
        )
        .expect("forced");

        assert_eq!(prepared.output, Some("partial".to_string()));
        assert_eq!(
            prepared.audit,
            Some(serde_json::json!({
                "completion_forced": {
                    "by": "human",
                    "blockers": [{ "kind": "turn_stopped" }],
                    "stop_cause": "user_stopped",
                    "without_turn": false,
                    "output_source": "turn_window",
                }
            }))
        );
    });
}

/// A forced completion never overrules a turn that changed after the snapshot was taken.
#[test]
fn forcing_a_changed_turn_still_reports_the_conflict() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "partial");
        write_settled_turn(&sessions_root, "session-1", "carry on", "final");
        store_snapshot(&pool, &node_run_id, &stopped_snapshot());

        let error = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::FORCE,
        )
        .expect_err("force cannot overrule a superseded turn");

        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeTurnChanged(ora_contracts::WorkflowNodeTurnChangedParams {
                node_id: "agent".to_string(),
                expected: Some(WorkflowNodeTurnKey {
                    user_seq: 0,
                    ended_seq: 2,
                }),
                actual: Some(WorkflowNodeTurnKey {
                    user_seq: 3,
                    ended_seq: 5,
                }),
            })
        );
    });
}

/// A forced completion never accepts a final message rewritten under the same turn.
#[test]
fn forcing_a_rewritten_output_still_reports_the_conflict() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "rewritten");
        let mut stored = stopped_snapshot();
        stored.blockers = Vec::new();
        store_snapshot(&pool, &node_run_id, &stored);

        let error = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::FORCE,
        )
        .expect_err("force cannot accept rewritten output");

        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeOutputChanged(
                ora_contracts::WorkflowNodeOutputChangedParams {
                    node_id: "agent".to_string(),
                    expected: stored.output_digest.clone(),
                    actual: Some(format!("sha256:{}", sha256_hex(b"rewritten"))),
                }
            )
        );
    });
}

/// A forced completion of a structured contract failure keeps the raw text, claims no structured
/// output, and records the blocker it overrode.
#[test]
fn forcing_a_structured_failure_keeps_raw_output_without_structured_output() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, _node_run_id, sessions_root) =
            parked_run_on(&temp, &pool, INTERACTIVE_STRUCTURED_GRAPH);
        write_settled_turn(&sessions_root, "session-1", "go", "not json");

        let prepared = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::FORCE,
        )
        .expect("force takes the current output");

        assert_eq!(prepared.output.as_deref(), Some("not json"));
        assert_eq!(prepared.structured_output, None);

        let audit = prepared.audit.expect("a forced completion is audited");
        let forced = &audit["completion_forced"];
        assert_eq!(forced["without_turn"], serde_json::json!(false));
        assert_eq!(forced["output_source"], serde_json::json!("turn_window"));
        let blockers = forced["blockers"].as_array().expect("blockers");
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0]["kind"], serde_json::json!("structured_invalid"));
        assert!(
            blockers[0]["params"]["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty()),
            "the audit keeps the validation message"
        );
    });
}

/// A settled turn with output is ready without materializing a second snapshot.
#[test]
fn an_existing_ready_snapshot_is_not_rewritten() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "done");
        let ready = CompletionSnapshot {
            user_seq: Some(0),
            turn_key: Some(TurnKey {
                user_seq: 0,
                ended_seq: 2,
            }),
            stop_reason: Some("end_turn".to_string()),
            turn_origin: Some(TurnOrigin::Human),
            stop_cause: None,
            park_kind: None,
            blockers: Vec::new(),
            warnings: Vec::new(),
            output_digest: Some(format!("sha256:{}", sha256_hex(b"done"))),
            provisional: false,
        };
        store_snapshot(&pool, &node_run_id, &ready);

        let prepared = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::PLAIN,
        )
        .expect("ready without a rewrite");

        assert_eq!(prepared.output, Some("done".to_string()));
        assert_eq!(stored_snapshot(&pool, &run_id), Some(ready));
    });
}

/// The history file is read once per prepare, which is what keeps a long turn affordable.
#[test]
fn a_missing_history_file_is_read_as_an_empty_turn() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, _node_run_id, sessions_root) = parked_run(&temp, &pool);
        assert!(!Path::new(&sessions_root).exists());

        let error = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::PLAIN,
        )
        .expect_err("an empty session has no turn to complete");

        assert!(matches!(
            error.public_error(),
            PublicError::WorkflowNodeIncomplete(_)
        ));
    });
}

/// A stored opening snapshot with a settled `EndTurn` turn, as the first park writes it.
fn opening_snapshot(final_text: &[u8]) -> CompletionSnapshot {
    CompletionSnapshot {
        user_seq: Some(0),
        turn_key: Some(TurnKey {
            user_seq: 0,
            ended_seq: 2,
        }),
        stop_reason: Some("end_turn".to_string()),
        turn_origin: Some(TurnOrigin::Workflow),
        stop_cause: None,
        park_kind: Some(ParkKind::AwaitingUser),
        blockers: vec![CompletionBlocker {
            kind: CompletionBlockerKind::TurnUnconfirmed,
            params: serde_json::Value::Null,
        }],
        warnings: Vec::new(),
        output_digest: Some(format!("sha256:{}", sha256_hex(final_text))),
        provisional: false,
    }
}

/// Confirming the opening turn is a normal completion: no forced audit and no re-derived blocker.
#[test]
fn confirming_the_opening_turn_completes_without_a_forced_audit() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "done");
        let opening = opening_snapshot(b"done");
        store_snapshot(&pool, &node_run_id, &opening);

        let prepared = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::CONFIRM,
        )
        .expect("the confirmed opening turn completes");

        assert_eq!(prepared.output, Some("done".to_string()));
        assert_eq!(
            prepared.audit, None,
            "confirmation is not a forced completion"
        );
        // The snapshot stays as the record of the park the user answered.
        assert_eq!(stored_snapshot(&pool, &run_id), Some(opening));
    });
}

/// Confirmation lifts only `turn_unconfirmed`: every other stored blocker still rejects it.
#[test]
fn confirmation_cannot_lift_other_blockers() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "done");
        let mut opening = opening_snapshot(b"done");
        opening.blockers.push(CompletionBlocker {
            kind: CompletionBlockerKind::OutputMissing,
            params: serde_json::Value::Null,
        });
        store_snapshot(&pool, &node_run_id, &opening);

        let error = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::CONFIRM,
        )
        .expect_err("the remaining blocker still rejects the completion");

        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeIncomplete(ora_contracts::WorkflowNodeIncompleteParams {
                node_id: "agent".to_string(),
                blockers: vec![WorkflowNodeCompletionBlocker {
                    kind: WorkflowNodeCompletionBlockerKind::OutputMissing,
                    message: None,
                }],
            }),
            "the already-confirmed marker is not listed again"
        );
    });
}

/// `force` cannot stand in for confirmation: the opening turn stays blocked and unforced.
#[test]
fn force_cannot_stand_in_for_confirmation() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "done");
        let opening = opening_snapshot(b"done");
        store_snapshot(&pool, &node_run_id, &opening);

        let error = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::FORCE,
        )
        .expect_err("force is not confirmation");

        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeIncomplete(ora_contracts::WorkflowNodeIncompleteParams {
                node_id: "agent".to_string(),
                blockers: vec![WorkflowNodeCompletionBlocker {
                    kind: WorkflowNodeCompletionBlockerKind::TurnUnconfirmed,
                    message: None,
                }],
            })
        );
        assert_eq!(stored_snapshot(&pool, &run_id), Some(opening));
    });
}

/// Confirmation on an already ready snapshot is an idempotent no-op.
#[test]
fn confirmation_is_a_noop_on_a_ready_snapshot() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        let ready = CompletionSnapshot {
            user_seq: Some(0),
            turn_key: Some(TurnKey {
                user_seq: 0,
                ended_seq: 2,
            }),
            stop_reason: Some("end_turn".to_string()),
            turn_origin: Some(TurnOrigin::Human),
            stop_cause: None,
            park_kind: None,
            blockers: Vec::new(),
            warnings: Vec::new(),
            output_digest: Some(format!("sha256:{}", sha256_hex(b"done"))),
            provisional: false,
        };
        write_settled_turn(&sessions_root, "session-1", "go", "done");
        store_snapshot(&pool, &node_run_id, &ready);

        let prepared = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval::CONFIRM,
        )
        .expect("confirmation does not block a ready turn");

        assert_eq!(prepared.output, Some("done".to_string()));
        assert_eq!(prepared.audit, None);
        assert_eq!(stored_snapshot(&pool, &run_id), Some(ready));
    });
}

/// A confirmed agent completion commits its approval beside any forced audit in one patch.
#[test]
fn confirmation_and_forced_audit_share_one_payload_patch() {
    let confirmation = CompletionConfirmation {
        by: ConfirmationBy::Human,
        at: 7,
    };
    assert_eq!(
        merge_completion_patch(
            None,
            Some(confirmation.clone()),
            NodeCompletionRequester::Human,
        ),
        Some(serde_json::json!({
            "confirmation": { "by": "human", "at": 7 },
            "requester": "human"
        }))
    );
    assert_eq!(
        merge_completion_patch(
            Some(serde_json::json!({ "completion_forced": { "blockers": [] } })),
            Some(confirmation),
            NodeCompletionRequester::Agent,
        ),
        Some(serde_json::json!({
            "completion_forced": { "blockers": [] },
            "confirmation": { "by": "human", "at": 7 },
            "requester": "agent"
        }))
    );
}
/// Confirming the accepted agent signal and the opening turn completes the first turn.
#[test]
fn confirming_an_agent_signal_completes_the_opening_turn() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "done");
        let snapshot = CompletionSnapshot {
            user_seq: Some(0),
            turn_key: Some(TurnKey {
                user_seq: 0,
                ended_seq: 2,
            }),
            stop_reason: Some("end_turn".to_string()),
            turn_origin: Some(TurnOrigin::Workflow),
            stop_cause: None,
            park_kind: None,
            blockers: vec![CompletionBlocker {
                kind: CompletionBlockerKind::TurnUnconfirmed,
                params: serde_json::Value::Null,
            }],
            warnings: Vec::new(),
            output_digest: Some(format!("sha256:{}", sha256_hex(b"done"))),
            provisional: false,
        };
        let signal = AgentSignal {
            signal_id: "signal-1".to_string(),
            requested_at: 1,
            turn_key: TurnKey {
                user_seq: 0,
                ended_seq: 2,
            },
        };
        store_snapshot_with_meta(
            &pool,
            &node_run_id,
            &snapshot,
            &CompletionMeta {
                policy: CompletionPolicy::Agent,
                agent_signal: Some(signal),
                confirmation: None,
                auto_accept: false,
            },
        );

        let prepared = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval {
                confirm_initial_turn: true,
                force: false,
                confirm_agent_completion: Some(signal_fingerprint("signal-1")),
                auto_confirm: false,
            },
        )
        .expect("confirming both the opening turn and the accepted signal completes it");

        assert_eq!(prepared.output, Some("done".to_string()));
        assert_eq!(
            prepared.confirmation.map(|confirmation| confirmation.by),
            Some(ConfirmationBy::Human)
        );
        assert_eq!(prepared.requester, NodeCompletionRequester::Agent);
    });
}

/// The agent signal confirmation cannot stand in for the opening turn's own confirmation.
#[test]
fn confirming_an_agent_signal_does_not_confirm_the_opening_turn() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "done");
        let snapshot = CompletionSnapshot {
            user_seq: Some(0),
            turn_key: Some(TurnKey {
                user_seq: 0,
                ended_seq: 2,
            }),
            stop_reason: Some("end_turn".to_string()),
            turn_origin: Some(TurnOrigin::Workflow),
            stop_cause: None,
            park_kind: None,
            blockers: vec![CompletionBlocker {
                kind: CompletionBlockerKind::TurnUnconfirmed,
                params: serde_json::Value::Null,
            }],
            warnings: Vec::new(),
            output_digest: Some(format!("sha256:{}", sha256_hex(b"done"))),
            provisional: false,
        };
        let signal = AgentSignal {
            signal_id: "signal-1".to_string(),
            requested_at: 1,
            turn_key: TurnKey {
                user_seq: 0,
                ended_seq: 2,
            },
        };
        store_snapshot_with_meta(
            &pool,
            &node_run_id,
            &snapshot,
            &CompletionMeta {
                policy: CompletionPolicy::Agent,
                agent_signal: Some(signal),
                confirmation: None,
                auto_accept: false,
            },
        );

        let error = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval {
                confirm_initial_turn: false,
                force: false,
                confirm_agent_completion: Some(signal_fingerprint("signal-1")),
                auto_confirm: false,
            },
        )
        .expect_err("the signal confirmation must not adopt the opening turn");

        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeIncomplete(ora_contracts::WorkflowNodeIncompleteParams {
                node_id: "agent".to_string(),
                blockers: vec![WorkflowNodeCompletionBlocker {
                    kind: WorkflowNodeCompletionBlockerKind::TurnUnconfirmed,
                    message: None,
                }],
            })
        );
    });
}

/// A headless auto-accept records `auto` attribution instead of a human confirmation.
#[test]
fn a_headless_auto_accept_records_auto_confirmation() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_run_id, sessions_root) = parked_run(&temp, &pool);
        write_settled_turn(&sessions_root, "session-1", "go", "done");
        let snapshot = CompletionSnapshot {
            user_seq: Some(0),
            turn_key: Some(TurnKey {
                user_seq: 0,
                ended_seq: 2,
            }),
            stop_reason: Some("end_turn".to_string()),
            turn_origin: Some(TurnOrigin::Human),
            stop_cause: None,
            park_kind: None,
            blockers: Vec::new(),
            warnings: Vec::new(),
            output_digest: Some(format!("sha256:{}", sha256_hex(b"done"))),
            provisional: false,
        };
        let signal = AgentSignal {
            signal_id: "signal-auto".to_string(),
            requested_at: 1,
            turn_key: TurnKey {
                user_seq: 0,
                ended_seq: 2,
            },
        };
        store_snapshot_with_meta(
            &pool,
            &node_run_id,
            &snapshot,
            &CompletionMeta {
                policy: CompletionPolicy::Agent,
                agent_signal: Some(signal),
                confirmation: None,
                auto_accept: true,
            },
        );

        let prepared = prepare_completion(
            &pool,
            &sessions_root,
            &run_id,
            "agent",
            CompletionApproval {
                confirm_initial_turn: false,
                force: false,
                confirm_agent_completion: Some(signal_fingerprint("signal-auto")),
                auto_confirm: true,
            },
        )
        .expect("headless auto-accept completes the settled turn");

        assert_eq!(
            prepared.confirmation.map(|confirmation| confirmation.by),
            Some(ConfirmationBy::Auto)
        );
    });
}
