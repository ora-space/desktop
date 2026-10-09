use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

/// Uses the production config decoder so wire contracts and round semantics are exercised together.
fn config() -> LoopConfig {
    LoopConfig::parse(json!({
        "maxIterations": 2,
        "variables": [
            {"name": "left", "valueType": "number", "initial": {"kind": "constant", "value": 1}, "feedback": ["loop", "right"]},
            {"name": "right", "valueType": "number", "initial": {"kind": "variable", "selector": ["start", "count"]}, "feedback": ["loop", "left"]}
        ],
        "until": {"logic": "and", "conditions": [
            {"variableSelector": ["review", "approved"], "operator": "is", "value": true}
        ]},
        "outputs": [{"name": "result", "variableSelector": ["writer", "structured", "text"]}]
    })).unwrap()
}

/// Builds a completed round with explicit declarations and values through the production pool.
fn pool() -> WorkflowVariablePool {
    let mut pool = WorkflowVariablePool::default();
    for (key, value_type, writer, value) in [
        ("start.count", "number", "start", json!(2)),
        ("loop.left", "number", "loop", json!(1)),
        ("loop.right", "number", "loop", json!(2)),
        ("review.approved", "boolean", "review", json!(false)),
        (
            "writer.structured",
            "object",
            "writer",
            json!({"text": "draft"}),
        ),
    ] {
        pool.declare(key, value_type, writer);
        pool.set(key, writer, value).unwrap();
    }
    pool
}

/// Initial references are resolved and checked without writing to the containing scope.
#[test]
fn initializes_literals_and_references() {
    let pool = pool();
    let before = pool.clone();
    assert_eq!(
        config().initialize_carried(&pool),
        Ok(BTreeMap::from([
            ("left".into(), json!(1)),
            ("right".into(), json!(2))
        ]))
    );
    assert_eq!(pool, before);
}

/// Swapping feedback inputs proves that assignment order cannot leak into another read.
#[test]
fn applies_feedback_simultaneously_without_mutation() {
    let pool = pool();
    let before = pool.clone();
    assert_eq!(
        config().complete_round(/*round*/ 1, &pool),
        Ok(LoopRoundDecision::Continue {
            carried: BTreeMap::from([("left".into(), json!(2)), ("right".into(), json!(1))])
        })
    );
    assert_eq!(pool, before);
}

/// Successful termination wins even at the exact cap, and named outputs resolve nested objects.
#[test]
fn terminates_on_first_or_last_round() {
    let mut pool = pool();
    pool.set("review.approved", "review", json!(true)).unwrap();
    for round in [1, 2] {
        assert_eq!(
            config().complete_round(round, &pool),
            Ok(LoopRoundDecision::Succeeded {
                outputs: BTreeMap::from([("result".into(), json!("draft"))])
            })
        );
    }
}

/// Exhaustion is distinct from success and never creates an extra round.
#[test]
fn enforces_exact_round_limit() {
    assert_eq!(
        config().complete_round(/*round*/ 2, &pool()),
        Err(LoopRoundError::LimitReached {
            max_iterations: 2,
            observed: "review.approved=false".to_string(),
        })
    );
    for round in [0, 3] {
        assert_eq!(
            config().complete_round(round, &pool()),
            Err(LoopRoundError::InvalidRound {
                round,
                max_iterations: 2
            })
        );
    }
}

/// Unset feedback, such as an inactive branch, aborts the whole transition without partial writes.
#[test]
fn missing_feedback_leaves_completed_pool_unchanged() {
    let mut pool = pool();
    pool.values.remove("loop.left");
    let before = pool.clone();
    assert_eq!(
        config().complete_round(/*round*/ 1, &pool),
        Err(LoopRoundError::UnsetValue {
            selector: "loop.left".into()
        })
    );
    assert_eq!(pool, before);
}

/// A source's own declaration does not make its value valid for a differently typed carried input.
#[test]
fn rejects_incompatible_initial_and_feedback_values() {
    let mut config = config();
    config.variables[1].value_type = "boolean".into();
    let expected = Err(LoopRoundError::TypeMismatch {
        name: "right".into(),
        value_type: "boolean".into(),
    });
    assert_eq!(config.initialize_carried(&pool()), expected);
    assert_eq!(
        config.complete_round(/*round*/ 1, &pool()),
        Err(LoopRoundError::TypeMismatch {
            name: "right".into(),
            value_type: "boolean".into()
        })
    );
}

/// Exports are resolved only at successful exit, while unresolved exit bindings fail explicitly.
#[test]
fn resolves_outputs_only_on_success() {
    let mut pool = pool();
    pool.values.remove("writer.structured");
    assert!(matches!(
        config().complete_round(/*round*/ 1, &pool),
        Ok(LoopRoundDecision::Continue { .. })
    ));
    pool.set("review.approved", "review", json!(true)).unwrap();
    assert_eq!(
        config().complete_round(/*round*/ 1, &pool),
        Err(LoopRoundError::UnsetValue {
            selector: "writer.structured.text".into()
        })
    );
}

/// Each round starts without child outputs and cannot import unrelated outer nodes.
#[test]
fn round_pools_isolate_child_outputs_and_inherited_writers() {
    let graph = WorkflowGraph::parse(&json!({
        "schemaVersion": 2,
        "nodes": [
            {"id": "start", "data": {"kind": "start"}},
            {"id": "other", "data": {"kind": "agent"}},
            {"id": "loop", "data": {"kind": "loop", "loopConfig": {
                "maxIterations": 2,
                "variables": [{"name": "draft", "valueType": "string", "initial": {"kind": "constant", "value": ""}, "feedback": ["writer", "output"]}],
                "until": {"logic": "and", "conditions": [{"variableSelector": ["writer", "output"], "operator": "equals", "value": "done"}]},
                "outputs": []
            }}},
            {"id": "entry", "data": {"kind": "start", "containerId": "loop"}},
            {"id": "writer", "data": {"kind": "agent", "containerId": "loop"}}
        ],
        "edges": [{"source": "start", "target": "loop"}, {"source": "start", "target": "other"}, {"source": "entry", "target": "writer"}]
    }).to_string()).unwrap();
    let mut outer = WorkflowVariablePool::from_graph(&graph);
    outer
        .set("start.input", "start", json!("requirement"))
        .unwrap();
    outer
        .set("other.output", "other", json!("unrelated"))
        .unwrap();
    let before = outer.clone();
    let carried = BTreeMap::from([("draft".into(), json!(""))]);
    let fresh = graph.loop_round_pool("loop", &outer, &carried).unwrap();
    assert_eq!(
        fresh.values,
        BTreeMap::from([
            ("start.input".into(), json!("requirement")),
            ("loop.draft".into(), json!(""))
        ])
    );
    assert!(!fresh.catalog.contains_key("other.output"));
    let mut completed = fresh.clone();
    completed
        .set("writer.output", "writer", json!("first round"))
        .unwrap();
    assert_eq!(
        graph.loop_round_pool("loop", &outer, &carried).unwrap(),
        fresh
    );
    assert_eq!(
        completed.set("start.input", "writer", json!("overwrite")),
        Err(WorkflowVariablePoolError::InvalidWriter {
            selector: "start.input".into(),
            expected_writer: "start".into(),
            actual_writer: "writer".into()
        })
    );
    assert_eq!(outer, before);
    assert_eq!(
        graph.loop_round_pool("loop", &outer, &BTreeMap::new()),
        Err(LoopRoundError::InvalidCarriedVariables)
    );
}
