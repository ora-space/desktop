use crate::workflow_run::engine::graph::{AgentOutputContract, WorkflowGraph, WorkflowGraphNode};
use crate::workflow_run::engine::iteration::IterationConfig;
use crate::workflow_run::engine::node_type::NodeType;
use crate::workflow_run::engine::variable_value::normalize_workflow_value;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

/// A declared workflow variable and the node that owns writes to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowVariableDefinition {
    pub value_type: String,
    pub writer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
}

/// The run-scoped variable pool persisted inside `workflow_runs.payload`.
///
/// Selectors use the Dify-style fully qualified form `{node_id}.{variable_name}`. The qualified
/// key keeps lookup unambiguous while still allowing different nodes to expose the same short
/// variable name, such as `agent-a.text` and `agent-b.text`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowVariablePool {
    pub revision: u64,
    pub catalog: BTreeMap<String, WorkflowVariableDefinition>,
    pub values: BTreeMap<String, Value>,
}

/// A fully-qualified reference to one workflow variable: `{node_id}.{root}` plus an optional
/// nested object path, serialized on the wire as `["node_id", "root", ...path]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableSelector {
    pub node_id: String,
    pub root: String,
    pub nested: Vec<String>,
}

impl VariableSelector {
    /// Builds a selector from its fully-qualified parts.
    pub fn new(node_id: String, root: String, nested: Vec<String>) -> Self {
        Self {
            node_id,
            root,
            nested,
        }
    }

    /// Parses the wire form `["node-id", "root", ...nested]`, rejecting empty node ids or roots.
    pub fn try_from_parts(parts: &[String]) -> Option<Self> {
        let mut parts = parts.iter();
        let node_id = parts.next()?;
        let root = parts.next()?;
        if node_id.is_empty() || root.is_empty() {
            return None;
        }
        Some(Self {
            node_id: node_id.clone(),
            root: root.clone(),
            nested: parts.cloned().collect(),
        })
    }

    /// The pool key, `{node_id}.{root}`, which is unique within one run.
    pub fn qualified(&self) -> String {
        format!("{}.{}", self.node_id, self.root)
    }
}

/// Errors raised when a node attempts to access or mutate the run variable pool.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkflowVariablePoolError {
    #[error("workflow variable is not declared: {selector}")]
    Undeclared { selector: String },
    #[error("workflow variable is owned by {expected_writer}, not {actual_writer}: {selector}")]
    InvalidWriter {
        selector: String,
        expected_writer: String,
        actual_writer: String,
    },
    #[error("workflow variable value does not match declared type {value_type}: {selector}")]
    TypeMismatch {
        selector: String,
        value_type: String,
    },
    #[error("workflow variable value exceeds maximum length {max_length}: {selector}")]
    LengthExceeded { selector: String, max_length: usize },
    #[error("workflow variable has no object path segment {segment}: {selector}")]
    PathMissing { selector: String, segment: String },
    #[error("workflow variable is not an object, so path {path} cannot be resolved: {selector}")]
    PathNotObject { selector: String, path: String },
}

/// Why an iteration node's selectors cannot be statically typed against the graph's variable
/// declarations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum IterationDeclarationError {
    #[error("iteration source {selector} is not declared")]
    IteratorUndeclared { selector: String },
    #[error("iteration source {selector} must be an array type, but is {value_type}")]
    IteratorNotArray {
        selector: String,
        value_type: String,
    },
    #[error("iteration source {selector} is produced inside the iteration's own region")]
    IteratorInsideOwnRegion { selector: String },
    #[error("collect target {selector} is not a node inside the iteration region")]
    CollectOutsideRegion { selector: String },
    #[error("collect target {selector} is not a declared variable")]
    CollectUndeclared { selector: String },
}

impl WorkflowVariablePool {
    /// Creates declarations for explicit Start inputs and node outputs in the current graph.
    ///
    /// Composite (iteration) nodes are declared in a second pass: their exposed types derive
    /// from the base catalog, and sequential iterations may iterate over an earlier iteration's
    /// exposed arrays, so they are processed in topological order.
    pub fn from_graph(graph: &WorkflowGraph) -> Self {
        let mut pool = Self::default();

        for variable in graph.global_variables() {
            let writer = variable.name.split('.').next().unwrap_or("global");
            pool.declare(&variable.name, &variable.value_type, writer);
            if let Some(value) = variable.value.clone() {
                pool.values.insert(variable.name.clone(), value);
            }
        }

        for node in graph.nodes() {
            declare_base_node_variables(&mut pool, node);
        }

        // Parsed graphs already passed `validate_iteration_declarations`, so declaration
        // failures here are impossible; skipping keeps `from_graph` total for hand-built
        // graphs in tests.
        let _ = declare_iteration_variables(&mut pool, graph);
        pool
    }

    /// Declares one fully qualified variable if it has not already been declared.
    pub fn declare(&mut self, selector: &str, value_type: &str, writer: &str) {
        self.catalog
            .entry(selector.to_string())
            .or_insert_with(|| WorkflowVariableDefinition {
                value_type: value_type.to_string(),
                writer: writer.to_string(),
                max_length: None,
            });
    }

    /// Reads a variable value without exposing the mutable backing map to callers.
    pub fn get(&self, selector: &str) -> Result<Option<&Value>, WorkflowVariablePoolError> {
        if !self.catalog.contains_key(selector) {
            return Err(WorkflowVariablePoolError::Undeclared {
                selector: selector.to_string(),
            });
        }
        Ok(self.values.get(selector))
    }

    /// Resolves a fully-qualified selector, walking nested object fields past the root variable.
    ///
    /// `Ok(None)` means the variable is declared but not yet assigned; a declared selector whose
    /// object path cannot be traversed is a hard error so callers fail instead of misreading.
    pub fn resolve(
        &self,
        selector: &VariableSelector,
    ) -> Result<Option<&Value>, WorkflowVariablePoolError> {
        let qualified = selector.qualified();
        if !self.catalog.contains_key(&qualified) {
            return Err(WorkflowVariablePoolError::Undeclared {
                selector: qualified,
            });
        }
        let Some(mut value) = self.values.get(&qualified) else {
            return Ok(None);
        };
        for segment in &selector.nested {
            match value {
                Value::Object(map) => {
                    value =
                        map.get(segment)
                            .ok_or_else(|| WorkflowVariablePoolError::PathMissing {
                                selector: qualified.clone(),
                                segment: segment.clone(),
                            })?;
                }
                _ => {
                    return Err(WorkflowVariablePoolError::PathNotObject {
                        selector: qualified.clone(),
                        path: selector.nested.join("."),
                    });
                }
            }
        }
        Ok(Some(value))
    }

    /// Writes a variable only through its declared owner and advances the mutation revision.
    pub fn set(
        &mut self,
        selector: &str,
        writer: &str,
        value: Value,
    ) -> Result<(), WorkflowVariablePoolError> {
        let definition =
            self.catalog
                .get(selector)
                .ok_or_else(|| WorkflowVariablePoolError::Undeclared {
                    selector: selector.to_string(),
                })?;
        if definition.writer != writer {
            return Err(WorkflowVariablePoolError::InvalidWriter {
                selector: selector.to_string(),
                expected_writer: definition.writer.clone(),
                actual_writer: writer.to_string(),
            });
        }
        let value = normalize_workflow_value(value, &definition.value_type).ok_or_else(|| {
            WorkflowVariablePoolError::TypeMismatch {
                selector: selector.to_string(),
                value_type: definition.value_type.clone(),
            }
        })?;
        if let (Some(max_length), Value::String(value)) = (definition.max_length, &value)
            && value.chars().count() > max_length
        {
            return Err(WorkflowVariablePoolError::LengthExceeded {
                selector: selector.to_string(),
                max_length,
            });
        }
        self.values.insert(selector.to_string(), value);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }
}

/// Declares one non-composite node's base variables: the shared `.output` for data-producing
/// nodes, Start inputs, and structured-output objects. Iteration nodes declare nothing here;
/// their exposed variables derive from these declarations in a second pass.
fn declare_base_node_variables(pool: &mut WorkflowVariablePool, node: &WorkflowGraphNode) {
    if !matches!(
        node.node_type,
        NodeType::Start | NodeType::Condition | NodeType::Iteration
    ) {
        pool.declare(&format!("{}.output", node.id), "string", &node.id);
    }
    match node.node_type {
        NodeType::Start => {
            // `{start_id}.input` is the reserved free-text selector for the run's kickoff
            // instruction. It matches the stable selector the editor catalog always exposes,
            // so a prompt may reference it even when no run has supplied text yet.
            pool.declare(&format!("{}.input", node.id), "string", &node.id);
            for variable in &node.input_variables {
                let selector = format!("{}.{}", node.id, variable.name);
                pool.declare(&selector, &variable.value_type, &node.id);
                if let Some(definition) = pool.catalog.get_mut(&selector) {
                    definition.max_length = variable.max_length;
                }
                if let Some(value) = variable.value.clone() {
                    pool.values.insert(selector, value);
                }
            }
        }
        NodeType::Agent => {
            if let Some(config) = node.agent_config.as_ref()
                && let Some(AgentOutputContract::Structured { .. }) =
                    config.output_contract.as_ref()
            {
                pool.declare(
                    &format!("{}.structured_output", node.id),
                    "object",
                    &node.id,
                );
            }
        }
        _ => {}
    }
}

/// Derives the element type an iteration's `{iter}.item` binds from the source array type
/// (`array[string]` → `string`; untyped arrays → `any`).
fn iteration_item_type(source_type: &str) -> &str {
    match source_type {
        "array" | "array[any]" => "any",
        "array[string]" => "string",
        "array[number]" => "number",
        "array[object]" => "object",
        "array[boolean]" => "boolean",
        "array[file]" => "file",
        _ => "any",
    }
}

/// Validates one iteration config against the working catalog and declares its five variables:
/// `item`, `index`, `output`, `entries`, and `failed_count` (ADR "iteration composite runtime"
/// D3). Called in topological order so a later iteration may iterate an earlier one's arrays.
fn declare_one_iteration(
    pool: &mut WorkflowVariablePool,
    graph: &WorkflowGraph,
    node_id: &str,
    config: &IterationConfig,
) -> Result<(), IterationDeclarationError> {
    let iterator_selector = config.iterator_selector.qualified();
    let source_type = pool
        .catalog
        .get(&iterator_selector)
        .map(|definition| definition.value_type.clone())
        .ok_or_else(|| IterationDeclarationError::IteratorUndeclared {
            selector: iterator_selector.clone(),
        })?;
    if !source_type.starts_with("array") {
        return Err(IterationDeclarationError::IteratorNotArray {
            selector: iterator_selector,
            value_type: source_type,
        });
    }
    let iterator_writer = pool
        .catalog
        .get(&iterator_selector)
        .map(|definition| definition.writer.clone())
        .unwrap_or_default();
    if graph
        .region(node_id)
        .is_some_and(|region| region.contains(&iterator_writer))
    {
        return Err(IterationDeclarationError::IteratorInsideOwnRegion {
            selector: iterator_selector,
        });
    }

    // The collect target must be a region member that declares the collected variable; its
    // declared type T fixes `{iter}.output` as `array[T]` for every error strategy.
    let collect_selector = config.collect_selector.qualified();
    if !graph
        .region(node_id)
        .is_some_and(|region| region.contains(&config.collect_selector.node_id))
    {
        return Err(IterationDeclarationError::CollectOutsideRegion {
            selector: collect_selector,
        });
    }
    let collect_type = pool
        .catalog
        .get(&collect_selector)
        .map(|definition| definition.value_type.clone())
        .ok_or(IterationDeclarationError::CollectUndeclared {
            selector: collect_selector,
        })?;

    let item_type = iteration_item_type(&source_type);
    pool.declare(&format!("{node_id}.item"), item_type, node_id);
    pool.declare(&format!("{node_id}.index"), "number", node_id);
    pool.declare(
        &format!("{node_id}.output"),
        &format!("array[{collect_type}]"),
        node_id,
    );
    pool.declare(&format!("{node_id}.entries"), "array[object]", node_id);
    pool.declare(&format!("{node_id}.failed_count"), "number", node_id);
    Ok(())
}

/// Declares every iteration node's exposed variables in topological order.
///
/// Returns the failing node's id together with the error so parse-time validation can point
/// the author at the offending iteration node.
fn declare_iteration_variables(
    pool: &mut WorkflowVariablePool,
    graph: &WorkflowGraph,
) -> Result<(), (String, IterationDeclarationError)> {
    for node in graph.nodes_in_topological_order() {
        if let Some(config) = node.iteration_config.as_ref()
            && let Err(error) = declare_one_iteration(pool, graph, &node.id, config)
        {
            return Err((node.id.clone(), error));
        }
    }
    Ok(())
}

/// Graph-parse-time validation of every iteration node's selectors against the graph's
/// derived variable declarations (ADR "iteration composite runtime" D1: statically decidable
/// at parse). Non-iteration graphs short-circuit so per-wave parsing pays nothing.
pub(super) fn validate_iteration_declarations(
    graph: &WorkflowGraph,
) -> Result<(), crate::workflow_run::engine::graph::GraphError> {
    if !graph
        .nodes()
        .any(|node| node.node_type == NodeType::Iteration)
    {
        return Ok(());
    }
    let mut pool = WorkflowVariablePool::default();
    for variable in graph.global_variables() {
        let writer = variable.name.split('.').next().unwrap_or("global");
        pool.declare(&variable.name, &variable.value_type, writer);
    }
    for node in graph.nodes() {
        declare_base_node_variables(&mut pool, node);
    }
    declare_iteration_variables(&mut pool, graph).map_err(|(node_id, error)| {
        crate::workflow_run::engine::graph::GraphError::InvalidIteration {
            node_id,
            reason: error.to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Allows identical short names on different nodes while keeping full selectors unique.
    #[test]
    fn selectors_are_qualified_by_node_id() {
        let mut pool = WorkflowVariablePool::default();
        pool.declare("agent-a.text", "string", "agent-a");
        pool.declare("agent-b.text", "string", "agent-b");

        pool.set("agent-a.text", "agent-a", Value::String("a".into()))
            .unwrap();
        pool.set("agent-b.text", "agent-b", Value::String("b".into()))
            .unwrap();

        assert_eq!(pool.values.len(), 2);
        assert_eq!(pool.revision, 2);
    }

    /// Rejects writes from a node that does not own the selected variable.
    #[test]
    fn rejects_writes_from_another_node() {
        let mut pool = WorkflowVariablePool::default();
        pool.declare("agent-a.text", "string", "agent-a");

        let error = pool
            .set("agent-a.text", "agent-b", Value::String("no".into()))
            .unwrap_err();

        assert_eq!(
            error,
            WorkflowVariablePoolError::InvalidWriter {
                selector: "agent-a.text".into(),
                expected_writer: "agent-a".into(),
                actual_writer: "agent-b".into(),
            }
        );
    }

    /// Rejects deployment values that do not satisfy the Start declaration's type.
    #[test]
    fn rejects_values_with_the_wrong_declared_type() {
        let mut pool = WorkflowVariablePool::default();
        pool.declare("start.count", "integer", "start");

        let error = pool
            .set(
                "start.count",
                "start",
                Value::String("not an integer".into()),
            )
            .unwrap_err();

        assert_eq!(
            error,
            WorkflowVariablePoolError::TypeMismatch {
                selector: "start.count".into(),
                value_type: "integer".into(),
            }
        );
    }

    /// Enforces Start text constraints again when deployment values enter the persisted pool.
    #[test]
    fn rejects_start_values_over_the_declared_maximum_length() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"start","data":{"kind":"start","inputVariables":[{
                        "name":"summary",
                        "displayName":"Summary",
                        "valueType":"string",
                        "maxLength":4
                    }]}}
                ],
                "edges": []
            }"#,
        )
        .unwrap();
        let mut pool = WorkflowVariablePool::from_graph(&graph);

        assert_eq!(
            pool.set("start.summary", "start", Value::String("longer".into())),
            Err(WorkflowVariablePoolError::LengthExceeded {
                selector: "start.summary".into(),
                max_length: 4,
            })
        );
    }

    /// File arrays migrate legacy path strings but reject mixed or unsafe entries.
    #[test]
    fn normalizes_and_validates_file_arrays() {
        let mut pool = WorkflowVariablePool::default();
        pool.declare("start.files", "array[file]", "start");

        pool.set(
            "start.files",
            "start",
            serde_json::json!(["one.txt", "nested/two.txt"]),
        )
        .unwrap();
        assert_eq!(
            pool.values.get("start.files"),
            Some(&serde_json::json!([
                { "kind": "workspace_file", "path": "one.txt" },
                { "kind": "workspace_file", "path": "nested/two.txt" }
            ]))
        );

        assert!(matches!(
            pool.set(
                "start.files",
                "start",
                serde_json::json!(["safe.txt", "../unsafe.txt"]),
            ),
            Err(WorkflowVariablePoolError::TypeMismatch { .. })
        ));
    }

    /// Data-producing nodes declare outputs while Conditions remain control-flow-only.
    #[test]
    fn graph_pool_excludes_condition_variables() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"start","data":{"kind":"start","inputVariables":[{"name":"limit","valueType":"integer"}]}},
                    {"id":"agent-1","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":""}}},
                    {"id":"condition-1","data":{"kind":"condition"}},
                    {"id":"output-1","data":{"kind":"output"}}
                ],
                "edges": []
            }"#,
        )
        .unwrap();

        assert_eq!(
            WorkflowVariablePool::from_graph(&graph).catalog,
            BTreeMap::from([
                (
                    "agent-1.output".to_string(),
                    WorkflowVariableDefinition {
                        value_type: "string".to_string(),
                        writer: "agent-1".to_string(),
                        max_length: None,
                    },
                ),
                (
                    "output-1.output".to_string(),
                    WorkflowVariableDefinition {
                        value_type: "string".to_string(),
                        writer: "output-1".to_string(),
                        max_length: None,
                    },
                ),
                (
                    "start.input".to_string(),
                    WorkflowVariableDefinition {
                        value_type: "string".to_string(),
                        writer: "start".to_string(),
                        max_length: None,
                    },
                ),
                (
                    "start.limit".to_string(),
                    WorkflowVariableDefinition {
                        value_type: "integer".to_string(),
                        writer: "start".to_string(),
                        max_length: None,
                    },
                ),
                (
                    "sys.timestamp".to_string(),
                    WorkflowVariableDefinition {
                        value_type: "number".to_string(),
                        writer: "sys".to_string(),
                        max_length: None,
                    },
                ),
                (
                    "sys.workflow_id".to_string(),
                    WorkflowVariableDefinition {
                        value_type: "string".to_string(),
                        writer: "sys".to_string(),
                        max_length: None,
                    },
                ),
            ])
        );
    }
}
