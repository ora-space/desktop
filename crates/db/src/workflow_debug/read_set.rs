//! Persist the facts a scoped plan depends on, including currently absent writers/attempts.
use super::*;
use serde_json::json;
use std::collections::BTreeSet;

/// Includes downstream consumers, their input producers and the body of each affected container.
/// A sibling reached only by following a producer's other outgoing edge is independent.
pub(super) fn nodes(graph: &WorkflowGraph, target: &str) -> BTreeSet<String> {
    use std::collections::BTreeMap;
    let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut inputs = BTreeMap::new();
    for scope in graph.execution_scopes() {
        for node in scope.nodes() {
            let mut producers = scope.input_producers(node);
            producers.extend(scope.predecessors(&node.id).iter().map(|n| n.id.clone()));
            if let Some(owner) = graph
                .loop_owner(&node.id)
                .or_else(|| graph.region_owner(&node.id))
            {
                producers.insert(owner.to_owned());
                dependents
                    .entry(node.id.clone())
                    .or_default()
                    .push(owner.to_owned());
            }
            for producer in &producers {
                dependents
                    .entry(producer.clone())
                    .or_default()
                    .push(node.id.clone());
            }
            inputs.insert(node.id.clone(), producers);
        }
    }
    let mut work = vec![target.to_owned()];
    let mut affected = BTreeSet::new();
    while let Some(node) = work.pop() {
        if affected.insert(node.clone()) {
            work.extend(dependents.remove(&node).unwrap_or_default());
        }
    }
    let mut work = Vec::from_iter(affected.iter().cloned());
    let mut reads = affected;
    while let Some(node) = work.pop() {
        for input in inputs.remove(&node).unwrap_or_default() {
            if reads.insert(input.clone()) {
                work.push(input);
            }
        }
    }
    reads
}

/// Compares declared-but-unassigned, explicit null and assigned values distinctly.
fn projection(mut state: Value, nodes: &BTreeSet<String>) -> Value {
    if let Some(pool) = state.get_mut("variablePool") {
        let mut selected = serde_json::Map::new();
        if let Some(catalog) = pool["catalog"].as_object() {
            for (key, definition) in catalog {
                if definition["writer"]
                    .as_str()
                    .is_some_and(|writer| nodes.contains(writer))
                {
                    let mut fact = json!({"definition":definition});
                    if let Some(value) = pool["values"].get(key) {
                        fact["value"] = value.clone();
                    }
                    selected.insert(key.clone(), fact);
                }
            }
        }
        *pool = Value::Object(selected);
    }
    for field in [
        "conditionDecisions",
        "iterationLedger",
        "iterationConditionDecisions",
    ] {
        if let Some(map) = state.get_mut(field).and_then(Value::as_object_mut) {
            map.retain(|k, _| {
                nodes.contains(k) || k.rsplit_once('#').is_some_and(|(id, _)| nodes.contains(id))
            });
        }
    }
    state
}

/// Called inside the same SQLite transaction as preview creation or operation admission.
pub(super) fn facts(db: &Connection, run: &str, encoded: &str) -> Result<Value, DatabaseError> {
    let nodes: BTreeSet<String> = serde_json::from_str(encoded)?;
    let (run_facts,payload) = db.query_row(
        "SELECT json_array(r.snapshot_id,root.scope_id,r.run_status,r.input,
           (SELECT MAX(id) FROM workflow_debug_operations WHERE run_id=r.id)),
           r.payload FROM workflow_runs r JOIN workflow_run_root_scopes root ON root.run_id=r.id WHERE r.id=?1",
        [run], |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?)))?;
    let versions: String = db.query_row(
        "SELECT json_group_object(node_id,version) FROM workflow_debug_node_versions
        WHERE run_id=?1 AND node_id IN (SELECT value FROM json_each(?2))",
        params![run, encoded],
        |r| r.get(0),
    )?;
    let mut stmt=db.prepare("SELECT s.id,s.parent_loop_node_run_id,s.round_index,s.status,s.state
        FROM workflow_execution_scopes s JOIN workflow_node_runs n ON n.id=s.parent_loop_node_run_id
        WHERE n.run_id=?1 AND n.is_deleted=0 AND n.node_id IN (SELECT value FROM json_each(?2)) ORDER BY s.id")?;
    let mut rows = stmt.query(params![run, encoded])?;
    let mut scopes = Vec::new();
    while let Some(row) = rows.next()? {
        let state: Value = serde_json::from_str(&row.get::<_, String>(4)?)?;
        scopes.push(json!([
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, u32>(2)?,
            row.get::<_, u32>(3)?,
            projection(state, &nodes)
        ]));
    }
    let payload = payload
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or(serde_json::to_value(
            ora_application::WorkflowRunPayload::default(),
        )?);
    Ok(json!([
        run_facts,
        projection(payload, &nodes),
        serde_json::from_str::<Value>(&versions)?,
        scopes
    ]))
}
