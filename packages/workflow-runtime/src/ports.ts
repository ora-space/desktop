import type {
  GraphWorkflowRun,
  GraphWorkflowSnapshotNodePatch,
  ProjectWorkflowMount,
  Unsubscribe,
  WorkflowArtifact,
  WorkflowDefinition,
  WorkflowEventCursor,
  WorkflowRunEventEnvelope,
  WorkflowRunLiveSnapshot,
} from "./types";

/**
 * Project-level binding of a workflow definition (reference, not a copy).
 *
 * Invariant: at most one mount per (projectId, definitionId). Remount refreshes
 * the stored definition blob. Multiple executions are GraphWorkflowRun rows, not
 * duplicate mounts.
 */
export interface WorkflowHostRepository {
  listMounts: (projectId: string) => Promise<ProjectWorkflowMount[]>;
  /** Projects that already reference this definition (for deploy UX grouping). */
  listMountsByDefinition: (
    definitionId: string,
  ) => Promise<ProjectWorkflowMount[]>;
  /** Registers or refreshes the definition blob, then upserts the project mount. */
  mount: (
    projectId: string,
    definition: WorkflowDefinition,
  ) => Promise<ProjectWorkflowMount>;
  unmount: (projectId: string, definitionId: string) => Promise<void>;
  getDefinition: (definitionId: string) => Promise<WorkflowDefinition | null>;
}

/** Lifecycle and event surface for GraphWorkflowRun instances. */
export interface WorkflowRunRepository {
  list: (projectId: string) => Promise<GraphWorkflowRun[]>;
  get: (runId: string) => Promise<GraphWorkflowRun | null>;
  create: (input: {
    projectId: string;
    definitionId: string;
    kickoffInput?: string;
  }) => Promise<GraphWorkflowRun>;
  /**
   * Starts a pending run. No-op when already running or terminal.
   * Create() does not auto-start by default; workspace Start calls this.
   */
  start: (runId: string) => Promise<GraphWorkflowRun>;
  cancel: (runId: string) => Promise<GraphWorkflowRun>;
  /**
   * Removes a run from the project list. Active runs are cancelled first so
   * concurrent siblings stay unaffected.
   */
  delete: (runId: string) => Promise<void>;
  /** Updates the display name shown in the sidebar and run workspace header. */
  rename: (runId: string, name: string) => Promise<GraphWorkflowRun>;
  /**
   * Patches copy fields on a node inside this run's frozen snapshot.
   * Only allowed while `pending` — never writes back to the mounted library
   * definition. Rejects once the run has started or finished.
   */
  updateSnapshotNode: (
    runId: string,
    nodeId: string,
    patch: GraphWorkflowSnapshotNodePatch,
  ) => Promise<GraphWorkflowRun>;
  submitHitl: (
    runId: string,
    requestId: string,
    payload: Record<string, unknown>,
  ) => Promise<GraphWorkflowRun>;
  listArtifacts: (runId: string) => Promise<WorkflowArtifact[]>;
  /** Returns an atomic run/artifact snapshot and the matching stream cursor. */
  getLiveSnapshot: (runId: string) => Promise<WorkflowRunLiveSnapshot | null>;
  /**
   * Subscribes to run events (node progress, artifacts, finish).
   *
   * Ordering contract: adapters must emit a single run's stream in strictly
   * increasing `sequence` order. Frontend projections rely on this (same as the
   * chat session stream) and only perform id-based upserts, not global re-sorts.
   * Callers must unregister on unmount.
   */
  subscribe: (
    runId: string,
    onEvent: (event: WorkflowRunEventEnvelope) => void,
    options?: { afterCursor?: WorkflowEventCursor | null },
  ) => Unsubscribe;
  /**
   * Fires whenever a run record mutates (engine steps, cancel, rename).
   * Used to invalidate react-query caches so sidebar status stays live.
   */
  watch: (onChange: (run: GraphWorkflowRun) => void) => Unsubscribe;
}

/** Combined runtime port so the shell can inject one memory or Desktop implementation. */
export interface WorkflowRuntime {
  host: WorkflowHostRepository;
  runs: WorkflowRunRepository;
  /** Releases adapter-owned timers, streams, and listeners. */
  dispose: () => void;
}
