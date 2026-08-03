import type { DemoWorkflow } from "@ora/workflow-mock";
import { createMockRunEngine } from "./mock-run-engine";
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
  WorkflowRunEvent,
} from "./types";

type Listener = (event: WorkflowRunEvent) => void;
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

function idleNodeStates(workflow: DemoWorkflow): Record<string, GraphWorkflowNodeState> {
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
  const definitions = new Map<string, DemoWorkflow>();
  const mounts: ProjectWorkflowMount[] = [];
  const runs = new Map<string, GraphWorkflowRun>();
  const artifacts = new Map<string, WorkflowArtifact[]>();
  const listeners = new Map<string, Set<Listener>>();
  const changeListeners = new Set<ChangeListener>();
  let runSeq = 0;
  let artifactSeq = 0;
  let hitlSeq = 0;

  const emit = (runId: string, event: WorkflowRunEvent) => {
    const set = listeners.get(runId);
    if (set === undefined) {
      return;
    }
    for (const listener of set) {
      listener(event);
    }
  };

  const notifyChanged = (run: GraphWorkflowRun) => {
    for (const listener of changeListeners) {
      listener(run);
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
        throw new Error(`Workflow ${definitionId} is not mounted on project ${projectId}`);
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
        totals: {},
        createdAt,
        updatedAt: createdAt,
      };
      runs.set(run.id, run);
      artifacts.set(run.id, []);
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
    },

    async cancel(runId) {
      const run = runs.get(runId);
      if (run === undefined) {
        throw new Error(`Unknown workflow run ${runId}`);
      }
      engine.cancel(runId);
    },

    async delete(runId) {
      const run = runs.get(runId);
      if (run === undefined) {
        return;
      }
      // Cancel in-flight work first; sibling runs keep their own state machines.
      if (
        run.status === "pending"
        || run.status === "running"
        || run.status === "awaiting_input"
      ) {
        engine.cancel(runId);
      } else {
        engine.stop(runId);
      }
      runs.delete(runId);
      artifacts.delete(runId);
      listeners.delete(runId);
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
    },

    async listArtifacts(runId) {
      return structuredClone(artifacts.get(runId) ?? []);
    },

    subscribe(runId, onEvent) {
      let set = listeners.get(runId);
      if (set === undefined) {
        set = new Set();
        listeners.set(runId, set);
      }
      set.add(onEvent);
      return () => {
        set.delete(onEvent);
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

  return { host, runs: runRepo };
}
