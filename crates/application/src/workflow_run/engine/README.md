# Workflow Run Engine

The execution engine that advances a `workflow_runs` row from `Pending` to a terminal state by
parsing the frozen snapshot graph, scheduling it as a DAG, and driving each node through the
runtime registered for its node type.

## Responsibilities

- **Graph parsing and topology** (`graph.rs`, `node_type.rs`): deserialize a frozen React Flow
  document into a validated `petgraph` DAG, validate structural invariants, and answer topology
  queries (full topological order, successors/predecessors, transitive closures, ready set,
  reachability).
- **Node runtimes and registry** (`node_runtime.rs`, `node_runtime/control.rs`): the per-node-type
  execution strategies behind a registry. Swift runtimes (`Start`, `Condition`, `Output`) complete
  synchronously inside a scheduling wave; the async Agent runtime wraps the backend's
  `NodeExecutor`. The registry is the single place a node type couples to an execution strategy:
  the scheduling core looks runtimes up by node type and dispatches on the registered execution
  form, never on the node type itself. Run-output precedence is runtime metadata too — each
  runtime declares the rank with which its succeeded nodes contribute the run's final output —
  so a new terminal node type picks its own precedence without touching the scheduling core.
- **Iteration composite runtime** (`iteration.rs`, `node_runtime/iteration.rs`): the first
  composite runtime (foreach semantics). `iteration.rs` owns the graph-level domain — the
  `iterationConfig` wire shape (`iteratorSelector`, `collectSelector`, `errorStrategy`,
  `maxIterations`), region derivation from React Flow `parentId` containment, the six region
  boundary rules validated at parse, and the per-round `RoundOutcome` ledger model with its
  `entries`/`output`/`failed_count` projection. `node_runtime/iteration.rs` is the pure advance
  planner: given the persisted rows, ledger, and pool, it answers with the next transition
  (`CompositeAdvancePlan`), which the engine executes through the repository's atomic composite
  operations. The engine holds no iteration state in memory — the current round is always
  re-derived from the region rows' `max(iteration)`.
- **Engine persistence port** (`ports.rs`): the `WorkflowRunEngineRepository` trait that the run
  engine uses, implemented in `ora-db`; plus the `WorkflowRunInvalidationPublisher` port that
  publishes a stateless invalidation after every committed run or node-run state transition. The
  port also carries the composite operations (`start_iteration_round`, `settle_iteration_round`,
  `complete_iteration_node`) and `FailurePropagation`: a failure inside a composite region
  resolves to the owning composite node (`Composite`), and the owner's error strategy decides at
  settlement whether the node fails (`fail`) or records the round and advances (`continue`).
- **Worktree initializer port** (`ports.rs`): the `WorkflowRunWorktreeInitializer` trait that the
  deploy flow calls to validate roles and resolve Effect-owned skill placements. It returns the
  actual per-node placements as a receipt rather than exposing a directory convention to later
  execution layers.
- **Skill delivery model** (`skill_delivery.rs`): Agent capability, non-empty validated discovery
  roots, frozen materialization bindings, and the typed workflow-run payload shared by deployment
  and node execution.
- **Branch projection** (`branch_projection.rs`): derives node states from persisted rows and
  Condition decisions. The outer projection treats composite regions as black boxes — members
  never enter the outer ready set and their per-round rows never seed outer states — while the
  scoped projection (`new_region_round`) schedules exactly one region round, consuming only that
  round's rows and per-round Condition decisions.
- **Run engine** (`engine.rs`): `start`/`cancel`/`restart` use cases and the reactive DAG scheduler.
  The scheduling core (`run_schedule`) recomputes state from persistence, hands in-flight nodes to
  their registered runtimes, advances composite nodes each wave, and finishes drained runs; it
  contains no node-type branching.

## Non-responsibilities

- Does not persist anything itself; it only defines the persistence port.
- Does not drive Ora sessions; agent execution is delegated through the `NodeExecutor` port that
  the engine wraps as the Agent node runtime.
- Does not resolve roles or materialize skills; role and skill binding validation is wired by the
  backend at deploy time through `WorkflowRunWorktreeInitializer`, while Effect independently owns
  physical Skill materialization. `start` therefore only validates graph executability.
- Does not run the workflow-run CRUD handlers (see the parent `workflow_run` module).
- Start-time graph-structural validation (`validate_executable_graph`, e.g. "output must be
  terminal", unique condition case ids) is graph policy like `WorkflowGraph::parse` itself — it
  inspects node kinds and therefore stays out of the runtime registry's reach.

## Public boundary

Exported from `workflow_run::engine`: `WorkflowRunEngine`, `WorkflowRunControlHandler`,
`NodeExecutor`, `WorkflowRunCallback`, `WorkflowRunEngineRepository`,
`WorkflowRunInvalidationPublisher`, `NoRunInvalidations`, `WorkflowGraph`,
`WorkflowGraphNode`, `AgentConfig`, `AgentExecutor`, `AgentSkill`, `AgentMcp`, `NodeType`,
`GraphError`, `UnknownNodeType`, `AgentSkillDeliveryProvider`, `SkillMaterializationReceipt`,
`WorkflowRunPayload`, and the repository outcome enums including
`BindWorkflowNodeSessionResult`. The node runtime traits and registry are internal to the engine
module; runtimes are registered by the engine's constructors.

## Module interactions

`ora-backend` implements `NodeExecutor` as `WorkflowRunNodeExecutor` and `WorkflowRunCallback` as
`WorkflowRunEngineCallback`, composing both in `build_workflow_run_engine` during `Backend::open`.
The engine wraps the executor as the Agent runtime and registers the swift control runtimes in
one assembly step. The backend also implements `WorkflowRunInvalidationPublisher` as a bridge
onto the application event hub: after every committed state transition the engine publishes one
`AppEvent::WorkflowRunInvalidated { run_id }`, which carries no workflow state — observers
re-query the persisted run. The interactive chain (an awaiting node parking, a human turn
beginning or ending) commits its guarded transitions through the backend's
`WorkflowRunTransitions` sink, which shares the same invalidation mechanism, so every committed
node-run transition — engine or interactive — publishes one event. `ora-db` implements
`WorkflowRunEngineRepository`. Agent-node sessions are a live path, not a test-only stub.

## Key invariants

- `WorkflowGraph` is immutable after `parse`; every topology query is deterministic.
- The graph is acyclic (validated by `petgraph::algo::toposort`), has unique node ids, and at most
  one start node; all three are rejected at parse time with a `GraphError` variant.
- The scheduling core is type-agnostic: the engine module and the registry's scheduling-facing
  surface contain no node-type literals outside the documented policy seams (registry assembly,
  registry lookups by parsed type, runtime implementations, start-time graph-structural
  validation), pinned by the `scheduling_core_contains_no_node_type_literals` source test that
  scans both modules rather than a single function. Adding a node type means adding a runtime
  plus one registration line in the registry assembly.
- Swift runtimes complete inside the scheduling wave and receive only in-memory committed facts —
  the `SwiftNodeRuntime` signature hands them no IO or persistence handle, and the
  `node_runtime_module_performs_no_io_or_waiting` source test rejects IO and waiting primitives
  anywhere in the runtime module, so the per-run serial gate is never held across a wait. Rust
  cannot make this a hard type-system guarantee, so the signature and the scan pin it together;
  bounded work remains a review obligation.
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
- Run invalidation events never carry workflow state: the persisted rows remain the only source
  of truth, and a lost or reordered event only leaves a stale view that the next event or refresh
  clears.
- Iteration regions are a structural property of the frozen graph: membership is `parentId`
  containment, validated by the six boundary rules at parse (non-empty and entered from the
  owner, no Output or nested composite inside, member out-edges stay inside, no outer node may
  target a member, `maxIterations >= 1`).
- Rounds are persisted facts: region rows carry their `iteration`, round variables
  (`item`/`index`) commit with the round's first rows in one transaction, ledger entries commit
  with their continuation in one transaction, and already-settled rounds are append-only. The
  exposed variables (`output: array[T]`, `entries: array[object]`, `failed_count: number`) are
  derived from the ledger at completion and never change type with the error strategy.
- An iteration node's own failures (non-array source, exceeded safety ceiling, exposed-variable
  write failures) always propagate to run failure; only failures inside the region can be
  absorbed by a `continue` strategy, and a branch that bypasses the collect target settles that
  round as failed instead of reading a stale pool value.

## Failure semantics

`GraphError` distinguishes structural failures: `InvalidJson`, `MissingNodes`, `MissingEdges`,
`InvalidNode`, `UnknownNodeType`, `DanglingEdge`, `CycleDetected`, `MultipleStartNodes`,
`DuplicateNodeId`, plus the iteration variants `InvalidIteration` (config or selector typing)
and `InvalidRegion` (region boundary violations). An empty graph is legal; unsupported-but-known
node types fail later at workflow start rather than at parse.

Agent MCP bindings are parsed and validated by `agent_config`; absent bindings default to an empty node allowlist. Runtime availability remains a Session setup responsibility.
