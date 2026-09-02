# Workflow Run Engine

The execution engine that advances a `workflow_runs` row from `Pending` to a terminal state by
parsing the frozen snapshot graph, scheduling it as a DAG, and driving each `agent` node through a
real Ora session.

## Responsibilities

- **Graph parsing and topology** (`graph.rs`, `node_type.rs`): deserialize a frozen React Flow
  document into a validated `petgraph` DAG, validate structural invariants, and answer topology
  queries (full topological order, successors/predecessors, transitive closures, ready set,
  reachability).
- **Engine persistence port** (`ports.rs`): the `WorkflowRunEngineRepository` trait that the run
  engine uses, implemented in `ora-db`.
- **Worktree initializer port** (`ports.rs`): the `WorkflowRunWorktreeInitializer` trait that the
  deploy flow calls to validate roles and resolve Effect-owned skill placements. It returns the
  actual per-node placements as a receipt rather than exposing a directory convention to later
  execution layers.
- **Skill delivery model** (`skill_delivery.rs`): Agent capability, non-empty validated discovery
  roots, frozen materialization bindings, and the typed workflow-run payload shared by deployment
  and node execution.
- **Run engine** (`engine.rs`): `start`/`cancel`/`restart` use cases, reactive DAG scheduling under
  a per-run serial executor, and the `NodeExecutor` port. `ora-backend` implements that port as
  `WorkflowRunNodeExecutor` and wires it in `Backend::open`.

## Non-responsibilities

- Does not persist anything itself; it only defines the persistence port.
- Does not drive Ora sessions; agent execution is delegated through `NodeExecutor`.
- Does not resolve roles or materialize skills; role and skill binding validation is wired by the
  backend at deploy time through `WorkflowRunWorktreeInitializer`, while Effect independently owns
  physical Skill materialization. `start` therefore only validates graph executability.
- Does not run the workflow-run CRUD handlers (see the parent `workflow_run` module).

## Public boundary

Exported from `workflow_run::engine`: `WorkflowRunEngine`, `WorkflowRunControlHandler`,
`NodeExecutor`, `WorkflowRunCallback`, `WorkflowRunEngineRepository`, `WorkflowGraph`,
`WorkflowGraphNode`, `AgentConfig`, `AgentExecutor`, `AgentSkill`, `NodeType`, `GraphError`,
`UnknownNodeType`, `AgentSkillDeliveryProvider`, `SkillMaterializationReceipt`,
`WorkflowRunPayload`, and the repository outcome enums including
`BindWorkflowNodeSessionResult`.

## Module interactions

`ora-backend` implements `NodeExecutor` as `WorkflowRunNodeExecutor` and `WorkflowRunCallback` as
`WorkflowRunEngineCallback`, composing both in `build_workflow_run_engine` during `Backend::open`.
`ora-db` implements `WorkflowRunEngineRepository`. Agent-node sessions are a live path, not a
test-only stub.

## Key invariants

- `WorkflowGraph` is immutable after `parse`; every topology query is deterministic.
- The graph is acyclic (validated by `petgraph::algo::toposort`), has unique node ids, and at most
  one start node; all three are rejected at parse time with a `GraphError` variant.
- Rust identifiers use `node_type` (aligned with `workflow_node_runs.node_type`); the wire source
  is React Flow's `data.kind`, read through a serde rename.
- Full-graph order and transitive closures use the same topological rank (upstream first), giving
  agent prompt assembly a stable panorama and input lineage.
- Skill discovery roots are validated worktree-relative paths supplied through an Agent capability
  provider. Deployment freezes the actual invocation name and package paths per node; execution
  does not re-resolve those values from the mutable global skill catalog.
- An agent node's `output` is its final assistant text. Complete conversation history belongs to
  the Ora session and is never duplicated into `workflow_node_runs`.
- A running agent node keeps `session_id` absent while its owning prompt is being prepared. The
  backend publishes the binding only after prompt admission, and the repository rejects that
  publication if cancellation or another terminal transition has already won.

## Failure semantics

`GraphError` distinguishes structural failures: `InvalidJson`, `MissingNodes`, `MissingEdges`,
`InvalidNode`, `UnknownNodeType`, `DanglingEdge`, `CycleDetected`, `MultipleStartNodes`, and
`DuplicateNodeId`. An empty graph is legal; unsupported-but-known node types fail later at
workflow start rather than at parse.
