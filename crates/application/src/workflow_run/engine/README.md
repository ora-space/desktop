# Workflow Run Engine

The execution engine that advances a `workflow_runs` row from `Pending` to a terminal state by
parsing the frozen snapshot graph, scheduling it as a DAG, and driving each `agent` node through a
real Ora session.

## Responsibilities

- **Graph parsing and topology** (`graph.rs`, `node_type.rs`): deserialize a frozen React Flow
  document into a validated `petgraph` DAG, validate structural invariants, and answer topology
  queries (successors/predecessors, transitive closures, ready set, reachability).
- **Engine persistence port** (`ports.rs`): the `WorkflowRunEngineRepository` trait that the run
  engine uses, implemented in `ora-db`.
- **Worktree initializer port** (`ports.rs`): the `WorkflowRunWorktreeInitializer` trait that the
  deploy flow calls to validate roles and materialize skills into a run worktree's initial state.
- **Run engine** (`engine.rs`): `start`/`cancel`/`restart` use cases, reactive DAG scheduling under
  a per-run serial executor, and the `NodeExecutor` port. `ora-backend` implements that port as
  `WorkflowRunNodeExecutor` and wires it in `Backend::open`.

## Non-responsibilities

- Does not persist anything itself; it only defines the persistence port.
- Does not drive Ora sessions; agent execution is delegated through `NodeExecutor`.
- Does not resolve roles or materialize skills; those are wired by the backend at deploy time
  (when the run worktree is created) through `WorkflowRunWorktreeInitializer`, so `start` only
  validates graph executability.
- Does not run the workflow-run CRUD handlers (see the parent `workflow_run` module).

## Public boundary

Exported from `workflow_run::engine`: `WorkflowRunEngine`, `WorkflowRunControlHandler`,
`NodeExecutor`, `WorkflowRunCallback`, `WorkflowRunEngineRepository`, `WorkflowGraph`,
`WorkflowGraphNode`, `AgentConfig`, `AgentExecutor`, `AgentSkill`, `NodeType`, `GraphError`,
`UnknownNodeType`.

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
- Transitive closures are ordered by topological rank (upstream first), giving agent prompt
  assembly a stable input lineage.

## Failure semantics

`GraphError` distinguishes structural failures: `InvalidJson`, `MissingNodes`, `MissingEdges`,
`InvalidNode`, `UnknownNodeType`, `DanglingEdge`, `CycleDetected`, `MultipleStartNodes`, and
`DuplicateNodeId`. An empty graph is legal; unsupported-but-known node types fail later at
workflow start rather than at parse.
