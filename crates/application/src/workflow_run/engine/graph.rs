use crate::workflow_run::engine::condition::{ConditionConfig, WireConditionCase};
use crate::workflow_run::engine::node_type::{NodeType, UnknownNodeType};
use crate::workflow_run::engine::structured_output::validate_structured_output_schema;
use crate::workflow_run::engine::variable_pool::VariableSelector;
use crate::workflow_run::engine::variable_value::{
    is_supported_variable_type, normalize_workflow_value,
};
use petgraph::Direction;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// One directed edge of a workflow graph, carrying the source port that selects its branch.
///
/// Plain nodes expose a single implicit `source` port; a Condition node exposes one port per case
/// plus the implicit `else` port, so the handle determines which downstream branch an edge feeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowGraphEdge {
    pub source: String,
    pub target: String,
    pub source_handle: Option<String>,
}

/// A parsed workflow execution graph.
///
/// Deserializes a frozen React Flow document (the snapshot graph) into a validated DAG. The
/// graph is immutable after construction; every topology query is deterministic.
#[derive(Debug, Clone)]
pub struct WorkflowGraph {
    graph: DiGraph<WorkflowGraphNode, Option<String>>,
    index_by_id: HashMap<String, NodeIndex>,
    /// Rank of each node in the unique `toposort` order, used to order transitive closures.
    topo_rank: HashMap<NodeIndex, usize>,
    global_variables: Vec<WorkflowGlobalVariable>,
}

/// One node in a parsed workflow graph.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowGraphNode {
    pub id: String,
    pub node_type: NodeType,
    pub title: String,
    pub description: String,
    /// Free-form instruction carried by `start`/`output` nodes (`data.instruction`).
    pub instruction: Option<String>,
    /// Typed variables declared by the Start node; empty for every other node.
    pub input_variables: Vec<StartInputVariable>,
    /// Execution contract of an `agent` node; absent for control nodes.
    pub agent_config: Option<AgentConfig>,
    /// Executable cases of a `condition` node; absent for non-condition nodes.
    pub condition_config: Option<ConditionConfig>,
    /// Declared result bindings of an `output` node; absent for non-output nodes.
    pub output_config: Option<OutputConfig>,
}

/// One typed variable declared by the Start node, optionally carrying its initial value.
#[derive(Debug, Clone, PartialEq)]
pub struct StartInputVariable {
    pub name: String,
    pub display_name: Option<String>,
    pub field_type: StartInputFieldType,
    pub value_type: String,
    pub required: bool,
    pub options: Vec<String>,
    pub max_length: Option<usize>,
    pub value: Option<serde_json::Value>,
}

/// Form control used to collect one Start variable without conflating UI and value types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartInputFieldType {
    TextInput,
    Paragraph,
    Select,
    Number,
    Checkbox,
    File,
    FileList,
    Json,
}

impl StartInputFieldType {
    /// Parses current field metadata or derives a compatible control for legacy snapshots.
    fn from_wire(field_type: Option<&str>, value_type: &str) -> Option<Self> {
        match field_type {
            Some("text-input") => Some(Self::TextInput),
            Some("paragraph") => Some(Self::Paragraph),
            Some("select") => Some(Self::Select),
            Some("number") => Some(Self::Number),
            Some("checkbox") => Some(Self::Checkbox),
            Some("file") => Some(Self::File),
            Some("file-list") => Some(Self::FileList),
            Some("json") => Some(Self::Json),
            Some(_) => None,
            None => match value_type {
                "number" | "integer" => Some(Self::Number),
                "boolean" => Some(Self::Checkbox),
                "file" => Some(Self::File),
                "array[file]" => Some(Self::FileList),
                "object" | "any" | "array" | "array[string]" | "array[number]"
                | "array[object]" | "array[boolean]" | "array[any]" => Some(Self::Json),
                "string" | "secret" => Some(Self::TextInput),
                _ => None,
            },
        }
    }

    /// Returns the exact variable-pool type emitted by current Start field controls.
    fn value_type(self) -> &'static str {
        match self {
            Self::TextInput | Self::Paragraph | Self::Select => "string",
            Self::Number => "number",
            Self::Checkbox => "boolean",
            Self::File => "file",
            Self::FileList => "array[file]",
            Self::Json => "object",
        }
    }
}

/// One workflow-wide variable declaration, independent of graph topology.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowGlobalVariable {
    pub name: String,
    pub value_type: String,
    pub value: Option<serde_json::Value>,
}

/// Whether an agent adds a structured variable beside its stable scalar `{node}.output`.
///
/// `Text` and `StructuredTextExposure` remain only to read snapshots written by the previous
/// contract shape; current graphs use `None` for text-only output and `Structured` for both.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentOutputContract {
    /// The node exposes only its stable `{node}.output` variable.
    None,
    /// Legacy spelling for a text-only node; its value is exposed as `{node}.output`.
    Text,
    /// The node exposes raw text as `{node}.output` and a validated object as
    /// `{node}.structured_output`.
    Structured {
        schema: serde_json::Value,
        text_exposure: StructuredTextExposure,
    },
}

/// Legacy structured-output text setting retained only for snapshot decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredTextExposure {
    /// Only `{node}.structured_output` is written; the raw text is withheld.
    StructuredOnly,
    /// Both `{node}.structured_output` and `{node}.text` are written.
    IncludeFinalText,
}

/// The result bindings of an `output` node, each resolving a variable selector to a named result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputConfig {
    pub outputs: Vec<OutputBinding>,
}

/// One named result an output node exposes, resolved from the run variable pool at completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputBinding {
    pub name: String,
    pub variable_selector: VariableSelector,
}

/// The executable contract of an `agent` node.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentConfig {
    pub executor: AgentExecutor,
    pub role_id: Option<String>,
    pub skills: Vec<AgentSkill>,
    pub prompt: String,
    /// When true the node is a persistent interactive session: its first turn pauses at
    /// `Pending` (awaiting input) instead of completing, and the user drives completion.
    pub interactive: bool,
    /// Optional structured parsing performed in addition to persisting the raw output.
    pub output_contract: Option<AgentOutputContract>,
}

/// The agent CLI and model an `agent` node must run with.
///
/// `agent_cli` stays a string here; validating it as an agent identity and checking
/// runtime availability happens in the session driver (phase 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentExecutor {
    pub agent_cli: String,
    pub model_id: String,
}

/// One skill an agent node declares; only `enabled` skills are materialized at start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSkill {
    pub skill_id: String,
    pub enabled: bool,
}

/// Structural failures discovered while deserializing and validating a frozen graph.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GraphError {
    #[error("workflow graph is not valid JSON")]
    InvalidJson,
    #[error("workflow graph is missing the nodes array")]
    MissingNodes,
    #[error("workflow graph is missing the edges array")]
    MissingEdges,
    #[error("invalid node: {reason}")]
    InvalidNode { reason: String },
    #[error("node {node_id} has unknown node type {value}")]
    UnknownNodeType { node_id: String, value: String },
    #[error("node {node_id} has an invalid condition config: {reason}")]
    InvalidCondition { node_id: String, reason: String },
    #[error("node {node_id} has invalid Start variables: {reason}")]
    InvalidStartVariables { node_id: String, reason: String },
    #[error("workflow has invalid global variables: {reason}")]
    InvalidGlobalVariables { reason: String },
    #[error("edge references missing node {node_id}")]
    DanglingEdge { node_id: String },
    #[error("workflow graph contains a cycle")]
    CycleDetected,
    #[error("workflow graph contains more than one start node")]
    MultipleStartNodes,
    #[error("duplicate node id: {node_id}")]
    DuplicateNodeId { node_id: String },
}

/// Wire shape of a frozen React Flow document.
///
/// Unknown top-level metadata fields (`id`, `name`, `description`, `updatedAt`, `viewport`) are
/// ignored by serde; only `nodes` and `edges` participate in execution.
#[derive(Debug, Deserialize)]
struct ReactFlowEnvelope {
    nodes: Option<Vec<WireNode>>,
    edges: Option<Vec<WireEdge>>,
    #[serde(default, alias = "globalVariables")]
    global_variables: Vec<WireGlobalVariable>,
}

/// Wire shape of one workflow-wide variable declaration.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireGlobalVariable {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    value_type: Option<String>,
    #[serde(default)]
    value: Option<serde_json::Value>,
}

/// Wire shape of one React Flow node. The renderer `type` is irrelevant to execution.
#[derive(Debug, Deserialize)]
struct WireNode {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    data: Option<WireNodeData>,
}

/// Wire shape of a node's `data` payload.
///
/// `kind` is the workflow node type on the wire; React Flow reserves the node-level `type` for
/// the renderer component, so the executable kind lives in `data`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireNodeData {
    #[serde(rename = "kind")]
    node_type: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    instruction: Option<String>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    input_variables: Vec<WireStartInputVariable>,
    #[serde(default)]
    agent_config: Option<WireAgentConfig>,
    #[serde(default)]
    cases: Vec<WireConditionCase>,
    #[serde(default)]
    outputs: Vec<WireOutputBinding>,
}

/// Wire shape of one typed Start input variable.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireStartInputVariable {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    field_type: Option<String>,
    #[serde(default)]
    value_type: Option<String>,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    options: Vec<String>,
    #[serde(default)]
    max_length: Option<usize>,
    #[serde(default)]
    value: Option<serde_json::Value>,
}

/// Wire shape of one `data.outputs` entry on an Output node.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireOutputBinding {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    variable_selector: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireAgentConfig {
    #[serde(default)]
    executor: Option<WireAgentExecutor>,
    #[serde(default)]
    role_id: Option<String>,
    #[serde(default)]
    skills: Vec<WireAgentSkill>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    interactive: Option<bool>,
    output_contract: Option<WireOutputContract>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireOutputContract {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    text_exposure: Option<String>,
    #[serde(default)]
    schema: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireAgentExecutor {
    #[serde(default)]
    agent_cli: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireAgentSkill {
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

/// Wire shape of one React Flow edge; the edge `id` is metadata and is ignored by serde.
///
/// `sourceHandle` names the source port feeding this edge: the implicit `source` port of a plain
/// node, a Condition's case id, or `else` for its fallback branch.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireEdge {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    source_handle: Option<String>,
}

impl WireAgentConfig {
    fn into_model(self) -> AgentConfig {
        AgentConfig {
            executor: AgentExecutor {
                agent_cli: self
                    .executor
                    .as_ref()
                    .and_then(|executor| executor.agent_cli.clone())
                    .unwrap_or_default(),
                model_id: self
                    .executor
                    .as_ref()
                    .and_then(|executor| executor.model_id.clone())
                    .unwrap_or_default(),
            },
            role_id: self.role_id,
            skills: self
                .skills
                .into_iter()
                .map(WireAgentSkill::into_model)
                .collect(),
            prompt: self.prompt.unwrap_or_default(),
            // Missing `interactive` defaults to false so existing graphs stay fully automatic.
            interactive: self.interactive.unwrap_or(false),
            output_contract: self
                .output_contract
                .and_then(WireOutputContract::into_model),
        }
    }
}

impl WireOutputContract {
    /// Maps the wire contract to the domain model; unknown kinds are ignored so future contract
    /// values parse as no contract on older Ora versions.
    fn into_model(self) -> Option<AgentOutputContract> {
        match self.kind.as_deref() {
            Some("none") => Some(AgentOutputContract::None),
            Some("text") => Some(AgentOutputContract::Text),
            Some("structured") => Some(AgentOutputContract::Structured {
                schema: self.schema.unwrap_or_default(),
                // Missing `textExposure` defaults to structured-only so the parsed object is the
                // authoritative variable unless the author opts the raw text back in.
                text_exposure: match self.text_exposure.as_deref() {
                    Some("includeFinalText") => StructuredTextExposure::IncludeFinalText,
                    _ => StructuredTextExposure::StructuredOnly,
                },
            }),
            _ => None,
        }
    }
}

/// Compiles `data.outputs` into an output config, skipping bindings with a missing name or an
/// unparseable selector; an empty result means the node keeps the legacy concatenated output.
fn into_output_config(wire: Vec<WireOutputBinding>) -> Option<OutputConfig> {
    let outputs: Vec<OutputBinding> = wire
        .into_iter()
        .filter_map(|binding| {
            Some(OutputBinding {
                name: binding.name?,
                variable_selector: VariableSelector::try_from_parts(&binding.variable_selector)?,
            })
        })
        .collect();
    (!outputs.is_empty()).then_some(OutputConfig { outputs })
}

/// Validates Start declarations before they become variable-pool catalog entries.
fn into_start_input_variables(
    wire: Vec<WireStartInputVariable>,
) -> Result<Vec<StartInputVariable>, String> {
    let mut names = HashSet::new();
    let mut variables = Vec::with_capacity(wire.len());
    for variable in wire {
        let name = variable.name.unwrap_or_default().trim().to_string();
        if name.is_empty() || name.contains('.') {
            return Err("variable names must be non-empty and cannot contain dots".into());
        }
        if !names.insert(name.clone()) {
            return Err(format!("duplicate variable name {name}"));
        }
        let value_type = variable.value_type.unwrap_or_default();
        if !is_supported_variable_type(&value_type) {
            return Err(format!("variable {name} has unsupported type {value_type}"));
        }
        let field_type =
            StartInputFieldType::from_wire(variable.field_type.as_deref(), &value_type)
                .ok_or_else(|| format!("variable {name} has unsupported Start field type"))?;
        if variable.field_type.is_some() && field_type.value_type() != value_type {
            return Err(format!(
                "variable {name} field type does not produce declared type {value_type}"
            ));
        }
        let options = variable
            .options
            .into_iter()
            .map(|option| option.trim().to_string())
            .collect::<Vec<_>>();
        if field_type == StartInputFieldType::Select
            && (options.is_empty()
                || options.iter().any(String::is_empty)
                || options.iter().collect::<HashSet<_>>().len() != options.len())
        {
            return Err(format!(
                "variable {name} select options must be non-empty and unique"
            ));
        }
        if field_type != StartInputFieldType::Select && !options.is_empty() {
            return Err(format!(
                "variable {name} options are only supported for select fields"
            ));
        }
        let display_name = variable
            .display_name
            .map(|display_name| display_name.trim().to_string())
            .filter(|display_name| !display_name.is_empty());
        let max_length = match variable.max_length {
            Some(0) => return Err(format!("variable {name} max length must be positive")),
            Some(_)
                if !matches!(
                    field_type,
                    StartInputFieldType::TextInput | StartInputFieldType::Paragraph
                ) =>
            {
                return Err(format!(
                    "variable {name} max length is only supported for text fields"
                ));
            }
            max_length => max_length,
        };
        let value = match variable.value {
            Some(value) => Some(normalize_workflow_value(value, &value_type).ok_or_else(|| {
                format!("variable {name} value does not match declared type {value_type}")
            })?),
            None => None,
        };
        if let (Some(max_length), Some(serde_json::Value::String(value))) = (max_length, &value)
            && value.chars().count() > max_length
        {
            return Err(format!(
                "variable {name} value exceeds maximum length {max_length}"
            ));
        }
        if field_type == StartInputFieldType::Select
            && value
                .as_ref()
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !options.iter().any(|option| option == value))
        {
            return Err(format!(
                "variable {name} initial value is not one of its select options"
            ));
        }
        variables.push(StartInputVariable {
            name,
            display_name,
            field_type,
            value_type,
            required: variable.required,
            options,
            max_length,
            value,
        });
    }
    Ok(variables)
}

/// Validates top-level global declarations and restores required runtime-owned variables.
fn into_global_variables(
    wire: Vec<WireGlobalVariable>,
) -> Result<Vec<WorkflowGlobalVariable>, String> {
    let mut variables = Vec::with_capacity(wire.len() + 2);
    let mut names = HashSet::new();
    for variable in wire {
        let name = variable.name.unwrap_or_default().trim().to_string();
        if name.split('.').count() < 2 || name.starts_with('.') || name.ends_with('.') {
            return Err(format!("global variable {name} must be a qualified name"));
        }
        if !names.insert(name.clone()) {
            return Err(format!("duplicate global variable {name}"));
        }
        let value_type = variable.value_type.unwrap_or_default();
        if !is_supported_variable_type(&value_type) {
            return Err(format!(
                "global variable {name} has unsupported type {value_type}"
            ));
        }
        // Only these two declarations are populated by the runtime. Requiring a value for every
        // user global prevents a graph that advertises a selectable variable but can never resolve it.
        let runtime_owned = matches!(name.as_str(), "sys.workflow_id" | "sys.timestamp");
        if !runtime_owned && variable.value.is_none() {
            return Err(format!("global variable {name} must have an initial value"));
        }
        let value = match variable.value {
            Some(value) => Some(normalize_workflow_value(value, &value_type).ok_or_else(|| {
                format!("global variable {name} value does not match declared type {value_type}")
            })?),
            None => None,
        };
        variables.push(WorkflowGlobalVariable {
            name,
            value_type,
            value,
        });
    }
    for (name, value_type) in [("sys.workflow_id", "string"), ("sys.timestamp", "number")] {
        variables.retain(|variable| variable.name != name);
        variables.push(WorkflowGlobalVariable {
            name: name.to_string(),
            value_type: value_type.to_string(),
            value: None,
        });
    }
    Ok(variables)
}

impl WireAgentSkill {
    fn into_model(self) -> AgentSkill {
        AgentSkill {
            skill_id: self.skill_id.unwrap_or_default(),
            // Missing `enabled` defaults to false so skills are never materialized by surprise.
            enabled: self.enabled.unwrap_or(false),
        }
    }
}

impl WorkflowGraph {
    /// Parses a frozen React Flow graph JSON into a validated DAG.
    pub fn parse(source: &str) -> Result<Self, GraphError> {
        let envelope: ReactFlowEnvelope =
            serde_json::from_str(source).map_err(|_| GraphError::InvalidJson)?;
        let wire_nodes = envelope.nodes.ok_or(GraphError::MissingNodes)?;
        let wire_edges = envelope.edges.ok_or(GraphError::MissingEdges)?;
        let global_variables = into_global_variables(envelope.global_variables)
            .map_err(|reason| GraphError::InvalidGlobalVariables { reason })?;

        let mut graph = DiGraph::<WorkflowGraphNode, Option<String>>::new();
        let mut index_by_id = HashMap::new();
        for wire_node in wire_nodes {
            let id = wire_node.id.ok_or_else(|| GraphError::InvalidNode {
                reason: "missing id".into(),
            })?;
            if index_by_id.contains_key(&id) {
                return Err(GraphError::DuplicateNodeId { node_id: id });
            }
            let data = wire_node.data.ok_or_else(|| GraphError::InvalidNode {
                reason: format!("node {id} has no data"),
            })?;
            let node_type = data
                .node_type
                .as_deref()
                .ok_or_else(|| GraphError::InvalidNode {
                    reason: format!("node {id} has no node type"),
                })?
                .parse::<NodeType>()
                .map_err(|UnknownNodeType(value)| GraphError::UnknownNodeType {
                    node_id: id.clone(),
                    value,
                })?;
            let node =
                WorkflowGraphNode {
                    id: id.clone(),
                    node_type,
                    title: data.title.unwrap_or_default(),
                    description: data.description.unwrap_or_default(),
                    // The editor stores a Start node's initial prompt in `data.input`; older
                    // snapshots used `data.instruction`. Prefer the current field, keep the legacy
                    // one as the fallback, and leave every other node on `instruction` untouched.
                    instruction: match node_type {
                        NodeType::Start => data.input.or(data.instruction),
                        _ => data.instruction,
                    },
                    input_variables: match node_type {
                        NodeType::Start => into_start_input_variables(data.input_variables)
                            .map_err(|reason| GraphError::InvalidStartVariables {
                                node_id: id.clone(),
                                reason,
                            })?,
                        _ => Vec::new(),
                    },
                    agent_config: data.agent_config.map(WireAgentConfig::into_model),
                    condition_config: match node_type {
                        NodeType::Condition => {
                            Some(ConditionConfig::from_wire(data.cases).map_err(|error| {
                                GraphError::InvalidCondition {
                                    node_id: id.clone(),
                                    reason: error.to_string(),
                                }
                            })?)
                        }
                        _ => None,
                    },
                    output_config: match node_type {
                        NodeType::Output => into_output_config(data.outputs),
                        _ => None,
                    },
                };
            if let Some(AgentOutputContract::Structured { schema, .. }) = node
                .agent_config
                .as_ref()
                .and_then(|config| config.output_contract.as_ref())
            {
                validate_structured_output_schema(schema).map_err(|error| {
                    GraphError::InvalidNode {
                        reason: format!(
                            "node {id} has an invalid structured output schema: {error}"
                        ),
                    }
                })?;
            }
            let index = graph.add_node(node);
            index_by_id.insert(id, index);
        }

        for wire_edge in wire_edges {
            let WireEdge {
                source,
                target,
                source_handle,
            } = wire_edge;
            let source = source.as_deref().ok_or_else(|| GraphError::DanglingEdge {
                node_id: target.clone().unwrap_or_default(),
            })?;
            let target = target.as_deref().ok_or_else(|| GraphError::DanglingEdge {
                node_id: source.to_string(),
            })?;
            let source_index =
                index_by_id
                    .get(source)
                    .copied()
                    .ok_or_else(|| GraphError::DanglingEdge {
                        node_id: source.to_string(),
                    })?;
            let target_index =
                index_by_id
                    .get(target)
                    .copied()
                    .ok_or_else(|| GraphError::DanglingEdge {
                        node_id: target.to_string(),
                    })?;
            graph.add_edge(source_index, target_index, source_handle);
        }

        let start_count = graph
            .node_weights()
            .filter(|node| node.node_type == NodeType::Start)
            .count();
        if start_count > 1 {
            return Err(GraphError::MultipleStartNodes);
        }

        let topo_order = toposort(&graph, None).map_err(|_| GraphError::CycleDetected)?;
        let topo_rank = topo_order
            .into_iter()
            .enumerate()
            .map(|(rank, index)| (index, rank))
            .collect();
        Ok(Self {
            graph,
            index_by_id,
            topo_rank,
            global_variables,
        })
    }

    /// Returns the unique start node, if the graph has one (parse guarantees at most one).
    pub fn start_node(&self) -> Option<&WorkflowGraphNode> {
        self.nodes().find(|node| node.node_type == NodeType::Start)
    }

    /// Returns the node with the given id, if present.
    pub fn node(&self, id: &str) -> Option<&WorkflowGraphNode> {
        self.index_by_id.get(id).map(|&index| &self.graph[index])
    }

    /// Iterates over every node in node-index (insertion) order.
    pub fn nodes(&self) -> impl Iterator<Item = &WorkflowGraphNode> {
        self.graph.node_weights()
    }

    /// Returns workflow-wide variable declarations, including required system globals.
    pub fn global_variables(&self) -> &[WorkflowGlobalVariable] {
        &self.global_variables
    }

    /// Returns the number of nodes.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Returns the number of edges.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Returns every node in deterministic topological execution order.
    pub fn nodes_in_topological_order(&self) -> Vec<&WorkflowGraphNode> {
        let mut indices: Vec<_> = self.graph.node_indices().collect();
        indices.sort_by_key(|index| self.topo_rank.get(index).copied().unwrap_or(usize::MAX));
        indices
            .into_iter()
            .map(|index| &self.graph[index])
            .collect()
    }

    /// Returns the direct successors of `id` in deterministic adjacency order.
    pub fn successors(&self, id: &str) -> Vec<&WorkflowGraphNode> {
        let Some(&index) = self.index_by_id.get(id) else {
            return Vec::new();
        };
        self.graph
            .neighbors_directed(index, Direction::Outgoing)
            .map(|neighbor| &self.graph[neighbor])
            .collect()
    }

    /// Returns the direct predecessors of `id` in deterministic adjacency order.
    pub fn predecessors(&self, id: &str) -> Vec<&WorkflowGraphNode> {
        let Some(&index) = self.index_by_id.get(id) else {
            return Vec::new();
        };
        self.graph
            .neighbors_directed(index, Direction::Incoming)
            .map(|neighbor| &self.graph[neighbor])
            .collect()
    }

    /// Returns the incoming edges of `id` with their source port handles, in deterministic order.
    pub fn incoming_edges(&self, id: &str) -> Vec<WorkflowGraphEdge> {
        let Some(&index) = self.index_by_id.get(id) else {
            return Vec::new();
        };
        let mut edges: Vec<_> = self
            .graph
            .edges_directed(index, Direction::Incoming)
            .map(|edge| WorkflowGraphEdge {
                source: self.graph[edge.source()].id.clone(),
                target: id.to_string(),
                source_handle: edge.weight().clone(),
            })
            .collect();
        // Petgraph exposes adjacency in insertion-independent order; sort so callers and tests
        // observe a stable sequence.
        edges.sort_by(|left, right| {
            (&left.source, &left.target, &left.source_handle).cmp(&(
                &right.source,
                &right.target,
                &right.source_handle,
            ))
        });
        edges
    }

    /// Returns the outgoing edges of `id` with their source port handles, in deterministic order.
    pub fn outgoing_edges(&self, id: &str) -> Vec<WorkflowGraphEdge> {
        let Some(&index) = self.index_by_id.get(id) else {
            return Vec::new();
        };
        let mut edges: Vec<_> = self
            .graph
            .edges_directed(index, Direction::Outgoing)
            .map(|edge| WorkflowGraphEdge {
                source: id.to_string(),
                target: self.graph[edge.target()].id.clone(),
                source_handle: edge.weight().clone(),
            })
            .collect();
        edges.sort_by(|left, right| {
            (&left.target, &left.source_handle).cmp(&(&right.target, &right.source_handle))
        });
        edges
    }

    /// Returns the transitive (downstream) closure of `id`, excluding the seed, in topological order.
    pub fn transitive_successors(&self, id: &str) -> Vec<&WorkflowGraphNode> {
        let Some(&seed) = self.index_by_id.get(id) else {
            return Vec::new();
        };
        self.order_by_topology(self.reachable_from(seed, Direction::Outgoing))
    }

    /// Returns the transitive (upstream) closure of `id`, excluding the seed, in topological order.
    pub fn transitive_predecessors(&self, id: &str) -> Vec<&WorkflowGraphNode> {
        let Some(&seed) = self.index_by_id.get(id) else {
            return Vec::new();
        };
        self.order_by_topology(self.reachable_from(seed, Direction::Incoming))
    }

    /// Returns nodes that are not yet completed and whose every direct predecessor is completed.
    ///
    /// A node with no direct predecessors (the start node) is vacuously ready.
    pub fn ready_set(&self, completed: &HashSet<&str>) -> Vec<&WorkflowGraphNode> {
        self.nodes()
            .filter(|node| !completed.contains(node.id.as_str()))
            .filter(|node| {
                self.predecessors(&node.id)
                    .iter()
                    .all(|predecessor| completed.contains(predecessor.id.as_str()))
            })
            .collect()
    }

    /// Returns the first node whose type v1 cannot execute, in node-index order.
    pub fn first_unsupported_node(&self) -> Option<&WorkflowGraphNode> {
        self.nodes().find(|node| !node.node_type.supported())
    }

    /// Returns the ids of nodes not reachable from the unique start node via directed edges.
    pub fn unreachable_from_start(&self) -> Vec<String> {
        let Some(start) = self.start_node() else {
            return self.nodes().map(|node| node.id.clone()).collect();
        };
        let reachable: HashSet<&str> = self
            .transitive_successors(&start.id)
            .iter()
            .map(|node| node.id.as_str())
            .collect();
        self.nodes()
            .filter(|node| node.id != start.id && !reachable.contains(node.id.as_str()))
            .map(|node| node.id.clone())
            .collect()
    }

    /// Collects every node reachable from `seed` following `direction` edges, excluding the seed.
    fn reachable_from(&self, seed: NodeIndex, direction: Direction) -> HashSet<NodeIndex> {
        let mut reached = HashSet::new();
        let mut stack = vec![seed];
        while let Some(current) = stack.pop() {
            for edge in self.graph.edges_directed(current, direction) {
                let neighbor = match direction {
                    Direction::Outgoing => edge.target(),
                    Direction::Incoming => edge.source(),
                };
                if reached.insert(neighbor) {
                    stack.push(neighbor);
                }
            }
        }
        reached
    }

    /// Sorts a set of node indices into the graph's topological order (upstream first).
    fn order_by_topology(&self, indices: HashSet<NodeIndex>) -> Vec<&WorkflowGraphNode> {
        let mut indices: Vec<_> = indices.into_iter().collect();
        indices.sort_by_key(|index| self.topo_rank.get(index).copied().unwrap_or(usize::MAX));
        indices
            .into_iter()
            .map(|index| &self.graph[index])
            .collect()
    }
}
