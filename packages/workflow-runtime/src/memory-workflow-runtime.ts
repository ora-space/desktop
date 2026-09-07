import { createMockRunEngine } from "./mock-run-engine";
import { validateWorkflowDefinition } from "./definition";
import type { MockPathPolicy } from "./mock-execution-plan";
import type {
  WorkflowHostRepository,
  WorkflowRunRepository,
  WorkflowRuntime,
} from "./ports";
import type {
  GraphWorkflowNodeState,
  GraphWorkflowRun,
  ProjectWorkflowMount,
  WorkflowArtifact,
  WorkflowNodeConversationItem,
  WorkflowDefinition,
  WorkflowRunEvent,
  WorkflowRunEventEnvelope,
} from "./types";

type Listener = (event: WorkflowRunEventEnvelope) => void;
type ChangeListener = (run: GraphWorkflowRun) => void;

export interface MemoryWorkflowRuntimeOptions {
  /** Delay between mock node steps. Default 5000ms (time to switch parallel acts). */
  nodeStepMs?: number;
  /**
   * When true, create() starts the mock engine immediately.
   * Default false: deploy only creates a pending run; workspace Start kicks off.
   */
  autoStart?: boolean;
  /** Injectable condition-branch policy for the mock engine. */
  pathPolicy?: MockPathPolicy;
  /** Locale for mock HITL schema strings. */
  locale?: "zh-CN" | "en-US";
  /** Receives observer failures without allowing UI code to stop the engine. */
  onListenerError?: (error: unknown) => void;
  /** Bounds replay memory while retaining enough events for UI setup races. */
  maxRetainedEvents?: number;
}

/** Local-time ISO timestamp for run metadata (Ora prefers local clocks). */
function nowIso(): string {
  const date = new Date();
  const pad = (value: number, width = 2) => String(value).padStart(width, "0");
  const offsetMin = -date.getTimezoneOffset();
  const sign = offsetMin >= 0 ? "+" : "-";
  const abs = Math.abs(offsetMin);
  const offset = `${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`;
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}.${pad(date.getMilliseconds(), 3)}${offset}`;
}

function idleNodeStates(
  workflow: WorkflowDefinition,
): Record<string, GraphWorkflowNodeState> {
  return Object.fromEntries(
    workflow.nodes.map((node) => [node.id, { status: "idle" as const }]),
  );
}

/**
 * In-memory Host + Run repositories for MVP.
 * Definition blobs live here after deploy; `@ora/workflow-mock` stays free of persistence.
 * The mock engine advances nodes on a timer and emits WorkflowRunEvent frames.
 */
export function createMemoryWorkflowRuntime(
  options: MemoryWorkflowRuntimeOptions = {},
): WorkflowRuntime {
  const autoStart = options.autoStart ?? false;
  const maxRetainedEvents = Math.max(1, options.maxRetainedEvents ?? 2_048);
  const definitions = new Map<string, WorkflowDefinition>();
  const mounts: ProjectWorkflowMount[] = [];
  const runs = new Map<string, GraphWorkflowRun>();
  const artifacts = new Map<string, WorkflowArtifact[]>();
  const conversations = new Map<string, WorkflowNodeConversationItem[]>();
  const listeners = new Map<string, Set<Listener>>();
  const changeListeners = new Set<ChangeListener>();
  const eventLogs = new Map<string, WorkflowRunEventEnvelope[]>();
  const eventSequences = new Map<string, number>();
  let runSeq = 0;
  let artifactSeq = 0;
  let hitlSeq = 0;
  let conversationItemSeq = 0;

  const emit = (runId: string, event: WorkflowRunEvent) => {
    const sequence = (eventSequences.get(runId) ?? 0) + 1;
    eventSequences.set(runId, sequence);
    const envelope: WorkflowRunEventEnvelope = {
      ...event,
      cursor: `${runId}:${sequence}`,
      sequence,
      occurredAt: nowIso(),
    };
    const log = eventLogs.get(runId) ?? [];
    log.push(envelope);
    if (log.length > maxRetainedEvents) {
      log.splice(0, log.length - maxRetainedEvents);
    }
    eventLogs.set(runId, log);
    const set = listeners.get(runId);
    if (set === undefined) {
      return;
    }
    for (const listener of set) {
      notifyListener(listener, envelope, options.onListenerError);
    }
  };

  const notifyChanged = (run: GraphWorkflowRun) => {
    for (const listener of changeListeners) {
      notifyListener(listener, run, options.onListenerError);
    }
  };

  const engine = createMockRunEngine(
    {
      getRun: (runId) => runs.get(runId),
      setRun: (run) => {
        runs.set(run.id, run);
      },
      appendArtifact: (artifact) => {
        const list = artifacts.get(artifact.runId) ?? [];
        list.push(artifact);
        artifacts.set(artifact.runId, list);
      },
      upsertConversationItem: (item) => {
        const list = conversations.get(item.runId) ?? [];
        const index = list.findIndex((current) => current.id === item.id);
        if (index < 0) {
          list.push(item);
        } else {
          list[index] = item;
        }
        conversations.set(item.runId, list);
      },
      emit,
      notifyChanged,
      nowIso,
      nextArtifactId: () => {
        artifactSeq += 1;
        return `wart-${artifactSeq}`;
      },
      nextHitlId: () => {
        hitlSeq += 1;
        return `hitl-${hitlSeq}`;
      },
      nextConversationItemId: () => {
        conversationItemSeq += 1;
        return `wconv-${conversationItemSeq}`;
      },
    },
    {
      nodeStepMs: options.nodeStepMs,
      pathPolicy: options.pathPolicy,
      locale: options.locale,
    },
  );

  const host: WorkflowHostRepository = {
    async listMounts(projectId) {
      return mounts
        .filter((mount) => mount.projectId === projectId)
        .map((mount) => structuredClone(mount));
    },

    async listMountsByDefinition(definitionId) {
      return mounts
        .filter((mount) => mount.definitionId === definitionId)
        .map((mount) => structuredClone(mount));
    },

    async mount(projectId, definition) {
      validateWorkflowDefinition(definition);
      definitions.set(definition.id, structuredClone(definition));
      const existing = mounts.findIndex(
        (mount) =>
          mount.projectId === projectId && mount.definitionId === definition.id,
      );
      const next: ProjectWorkflowMount = {
        projectId,
        definitionId: definition.id,
        definitionName: definition.name,
        mountedAt: nowIso(),
      };
      if (existing >= 0) {
        mounts[existing] = next;
      } else {
        mounts.push(next);
      }
      return structuredClone(next);
    },

    async unmount(projectId, definitionId) {
      const index = mounts.findIndex(
        (mount) =>
          mount.projectId === projectId && mount.definitionId === definitionId,
      );
      if (index >= 0) {
        mounts.splice(index, 1);
      }
    },

    async getDefinition(definitionId) {
      const definition = definitions.get(definitionId);
      return definition === undefined ? null : structuredClone(definition);
    },
  };

  const runRepo: WorkflowRunRepository = {
    async list(projectId) {
      return [...runs.values()]
        .filter((run) => run.projectId === projectId)
        .map((run) => structuredClone(run))
        .sort((left, right) => right.createdAt.localeCompare(left.createdAt));
    },

    async get(runId) {
      const run = runs.get(runId);
      return run === undefined ? null : structuredClone(run);
    },

    async create({ projectId, definitionId, kickoffInput }) {
      const mounted = mounts.some(
        (mount) =>
          mount.projectId === projectId && mount.definitionId === definitionId,
      );
      if (!mounted) {
        throw new Error(
          `Workflow ${definitionId} is not mounted on project ${projectId}`,
        );
      }
      const definition = definitions.get(definitionId);
      if (definition === undefined) {
        throw new Error(`Unknown workflow definition ${definitionId}`);
      }
      // Freeze the graph so later library edits cannot rewrite this run.
      const snapshot = structuredClone(definition);
      runSeq += 1;
      const createdAt = nowIso();
      const run: GraphWorkflowRun = {
        id: `gwr-${runSeq}`,
        projectId,
        definitionId,
        definitionSnapshot: snapshot,
        name: snapshot.name,
        status: "pending",
        kickoffInput,
        nodeStates: idleNodeStates(snapshot),
        openHitls: [],
        createdAt,
        updatedAt: createdAt,
      };
      runs.set(run.id, run);
      artifacts.set(run.id, []);
      conversations.set(run.id, []);
      eventLogs.set(run.id, []);
      eventSequences.set(run.id, 0);
      if (autoStart) {
        engine.start(run.id);
      }
      const current = runs.get(run.id)!;
      notifyChanged(current);
      return structuredClone(current);
    },

    async start(runId) {
      const run = runs.get(runId);
      if (run === undefined) {
        throw new Error(`Unknown workflow run ${runId}`);
      }
      engine.start(runId);
      return structuredClone(runs.get(runId)!);
    },

    async cancel(runId) {
      const run = runs.get(runId);
      if (run === undefined) {
        throw new Error(`Unknown workflow run ${runId}`);
      }
      engine.cancel(runId);
      return structuredClone(runs.get(runId)!);
    },

    async delete(runId) {
      const run = runs.get(runId);
      if (run === undefined) {
        return;
      }
      // Cancel in-flight work first; sibling runs keep their own state machines.
      if (
        run.status === "pending" ||
        run.status === "running" ||
        run.status === "awaiting_input"
      ) {
        engine.cancel(runId);
      } else {
        engine.stop(runId);
      }
      runs.delete(runId);
      artifacts.delete(runId);
      conversations.delete(runId);
      listeners.delete(runId);
      eventLogs.delete(runId);
      eventSequences.delete(runId);
    },

    async rename(runId, name) {
      const run = runs.get(runId);
      if (run === undefined) {
        throw new Error(`Unknown workflow run ${runId}`);
      }
      const trimmed = name.trim();
      if (trimmed === "") {
        throw new Error("Workflow run name cannot be empty");
      }
      const updated: GraphWorkflowRun = {
        ...run,
        name: trimmed,
        updatedAt: nowIso(),
      };
      runs.set(runId, updated);
      notifyChanged(updated);
      return structuredClone(updated);
    },

    async updateSnapshotNode(runId, nodeId, patch) {
      const run = runs.get(runId);
      if (run === undefined) {
        throw new Error(`Unknown workflow run ${runId}`);
      }
      if (run.status !== "pending") {
        throw new Error(
          `Snapshot node edits require pending status (got ${run.status})`,
        );
      }
      const nodeIndex = run.definitionSnapshot.nodes.findIndex(
        (node) => node.id === nodeId,
      );
      if (nodeIndex < 0) {
        throw new Error(`Unknown snapshot node ${nodeId}`);
      }
      const node = run.definitionSnapshot.nodes[nodeIndex]!;
      const nextData = { ...node.data };
      if (patch.description !== undefined) {
        nextData.description = patch.description;
      }
      if (patch.input !== undefined) {
        nextData.input = patch.input;
      }
      if (patch.instruction !== undefined) {
        nextData.instruction = patch.instruction;
      }
      const nextNodes = run.definitionSnapshot.nodes.slice();
      nextNodes[nodeIndex] = {
        ...node,
        data: nextData,
      };
      const updated: GraphWorkflowRun = {
        ...run,
        definitionSnapshot: {
          ...run.definitionSnapshot,
          nodes: nextNodes,
          updatedAt: nowIso(),
        },
        updatedAt: nowIso(),
      };
      runs.set(runId, updated);
      notifyChanged(updated);
      return structuredClone(updated);
    },

    async submitHitl(runId, requestId, payload) {
      const run = runs.get(runId);
      if (run === undefined) {
        throw new Error(`Unknown workflow run ${runId}`);
      }
      engine.submitHitl(runId, requestId, payload);
      return structuredClone(runs.get(runId)!);
    },

    async listArtifacts(runId) {
      return structuredClone(artifacts.get(runId) ?? []);
    },

    async getLiveSnapshot(runId) {
      const run = runs.get(runId);
      if (run === undefined) {
        return null;
      }
      const log = eventLogs.get(runId) ?? [];
      return {
        run: structuredClone(run),
        artifacts: structuredClone(artifacts.get(runId) ?? []),
        conversation: structuredClone(conversations.get(runId) ?? []),
        cursor: log.at(-1)?.cursor ?? null,
      };
    },

    subscribe(runId, onEvent, subscribeOptions = {}) {
      const afterCursor = subscribeOptions.afterCursor;
      const queuedLiveEvents: WorkflowRunEventEnvelope[] = [];
      let replaying = true;
      const liveListener: Listener = (event) => {
        const clone = structuredClone(event);
        if (replaying) {
          queuedLiveEvents.push(clone);
          return;
        }
        notifyListener(onEvent, clone, options.onListenerError);
      };
      let set = listeners.get(runId);
      if (set === undefined) {
        set = new Set();
        listeners.set(runId, set);
      }
      // Register before replay so an observer-triggered synchronous mutation
      // cannot land in the snapshot-to-live handoff gap.
      set.add(liveListener);
      if ("afterCursor" in subscribeOptions) {
        const log = eventLogs.get(runId) ?? [];
        const cursorIndex =
          afterCursor === null
            ? -1
            : log.findIndex((event) => event.cursor === afterCursor);
        const replayFrom =
          afterCursor === null || cursorIndex < 0 ? 0 : cursorIndex + 1;
        for (const event of log.slice(replayFrom)) {
          notifyListener(
            onEvent,
            structuredClone(event),
            options.onListenerError,
          );
        }
      }
      replaying = false;
      for (const event of queuedLiveEvents) {
        notifyListener(onEvent, event, options.onListenerError);
      }
      return () => {
        set.delete(liveListener);
        if (set.size === 0) {
          listeners.delete(runId);
        }
      };
    },

    watch(onChange) {
      changeListeners.add(onChange);
      return () => {
        changeListeners.delete(onChange);
      };
    },
  };

  return {
    host,
    runs: runRepo,
    dispose() {
      engine.dispose();
      listeners.clear();
      changeListeners.clear();
      eventLogs.clear();
      eventSequences.clear();
      conversations.clear();
    },
  };
}

/** Isolates observer failures so one consumer cannot stop workflow progression. */
function notifyListener<T>(
  listener: (value: T) => void,
  value: T,
  onListenerError: ((error: unknown) => void) | undefined,
): void {
  try {
    listener(value);
  } catch (error) {
    onListenerError?.(error);
  }
}
