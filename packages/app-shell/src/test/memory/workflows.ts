import {
  type Workflow,
  type WorkflowSnapshot,
  type WorkflowSummary,
  type WorkflowVersion,
} from "@ora/contracts";
import type { TestHandlers } from "../contracts-transport";
import { nextTimestamp, nextId } from "./records";

/** Mutable records owned by the workflows test adapter. */
export interface WorkflowMemoryState {
  workflows: MockWorkflowRecord[];
}

/** One in-memory workflow with its editable draft and published history. */
export interface MockWorkflowRecord {
  workflow: Workflow;
  draft: WorkflowSnapshot;
  published: WorkflowSnapshot[];
}

/** Creates an independent workflows memory fixture. */
export function createWorkflowMemory(): WorkflowMemoryState {
  return { workflows: [] };
}

/** Returns one workflow record or fails like the real not-found endpoint. */
function requireWorkflowRecord(
  state: WorkflowMemoryState,
  workflowId: string,
): MockWorkflowRecord {
  const record = state.workflows.find(
    (candidate) => candidate.workflow.id === workflowId,
  );
  if (record === undefined) {
    throw new Error(`workflow ${workflowId} not found`);
  }
  return record;
}

/** Registers only the workflows operations explicitly requested by a fixture. */
export function workflowHandlers(state: WorkflowMemoryState) {
  return {
    createWorkflow: async (req) => {
      const now = nextTimestamp();
      const id = nextId("wf", state.workflows.length);
      const workflow: Workflow = {
        id,
        namespace: "local",
        name: req.name,
        publishedSnapshotId: null,
        createdAt: now,
        updatedAt: now,
      };
      const draft: WorkflowSnapshot = {
        id: nextId("snap", 0),
        workflowId: id,
        version: "draft",
        graph: req.graph ?? "{}",
        createdAt: now,
        updatedAt: now,
      };
      state.workflows.unshift({ workflow, draft, published: [] });
      return { workflow, draft };
    },
    getWorkflow: async (req) => {
      const record = requireWorkflowRecord(state, req.workflowId);
      const published =
        record.workflow.publishedSnapshotId == null
          ? null
          : (record.published.find(
              (item) => item.id === record.workflow.publishedSnapshotId,
            ) ?? null);
      return {
        workflow: record.workflow,
        draft: record.draft,
        published,
      };
    },
    listWorkflows: async () => ({
      workflows: state.workflows.map((record): WorkflowSummary => ({
        id: record.workflow.id,
        namespace: record.workflow.namespace,
        name: record.workflow.name,
        publishedVersion:
          record.workflow.publishedSnapshotId == null
            ? null
            : (record.published.find(
                (item) => item.id === record.workflow.publishedSnapshotId,
              )?.version ?? null),
        createdAt: record.workflow.createdAt,
        updatedAt: record.workflow.updatedAt,
      })),
    }),
    updateWorkflow: async (req) => {
      const record = requireWorkflowRecord(state, req.workflowId);
      record.workflow = {
        ...record.workflow,
        name: req.name,
        updatedAt: nextTimestamp(),
      };
      return { workflow: record.workflow };
    },
    deleteWorkflow: async (req) => {
      const idx = state.workflows.findIndex(
        (record) => record.workflow.id === req.workflowId,
      );
      if (idx >= 0) state.workflows.splice(idx, 1);
      return { workflowId: req.workflowId };
    },
    getDraft: async (req) => {
      const record = requireWorkflowRecord(state, req.workflowId);
      return { snapshot: record.draft };
    },
    updateDraft: async (req) => {
      const record = requireWorkflowRecord(state, req.workflowId);
      record.draft = {
        ...record.draft,
        graph: req.graph,
        updatedAt: nextTimestamp(),
      };
      return { snapshot: record.draft };
    },
    publishWorkflow: async (req) => {
      const record = requireWorkflowRecord(state, req.workflowId);
      const now = nextTimestamp();
      const version = req.version ?? `v${now}`;
      const snapshot: WorkflowSnapshot = {
        id: nextId("snap", record.published.length),
        workflowId: record.workflow.id,
        version,
        graph: record.draft.graph,
        createdAt: now,
        updatedAt: null,
      };
      record.published.push(snapshot);
      record.workflow = {
        ...record.workflow,
        publishedSnapshotId: snapshot.id,
        updatedAt: now,
      };
      return { snapshot };
    },
    rollbackWorkflow: async (req) => {
      const record = requireWorkflowRecord(state, req.workflowId);
      const all = [...record.published, record.draft];
      const snapshot = all.find((item) => item.id === req.snapshotId);
      if (snapshot === undefined)
        throw new Error(`snapshot ${req.snapshotId} not found`);
      record.draft = {
        ...record.draft,
        graph: snapshot.graph,
        updatedAt: nextTimestamp(),
      };
      return { snapshot: record.draft };
    },
    activateWorkflow: async (req) => {
      const record = requireWorkflowRecord(state, req.workflowId);
      const snapshot = record.published.find(
        (item) => item.id === req.snapshotId,
      );
      if (snapshot === undefined)
        throw new Error(`snapshot ${req.snapshotId} not found`);
      record.workflow = {
        ...record.workflow,
        publishedSnapshotId: snapshot.id,
        updatedAt: nextTimestamp(),
      };
      record.draft = {
        ...record.draft,
        graph: snapshot.graph,
        updatedAt: nextTimestamp(),
      };
      return { snapshot: record.draft };
    },
    listVersions: async (req) => {
      const record = requireWorkflowRecord(state, req.workflowId);
      return {
        versions: record.published.map((snapshot): WorkflowVersion => ({
          id: snapshot.id,
          version: snapshot.version,
          createdAt: snapshot.createdAt,
        })),
      };
    },
    getVersion: async (req) => {
      const record = requireWorkflowRecord(state, req.workflowId);
      const snapshot = record.published.find(
        (item) => item.version === req.version,
      );
      if (snapshot === undefined)
        throw new Error(`snapshot ${req.version} not found`);
      return { snapshot };
    },
    deleteSnapshot: async (req) => {
      const record = requireWorkflowRecord(state, req.workflowId);
      const idx = record.published.findIndex(
        (item) => item.version === req.version,
      );
      if (idx < 0) throw new Error(`snapshot ${req.version} not found`);
      const [removed] = record.published.splice(idx, 1);
      return { snapshotId: removed.id, version: req.version };
    },
    getWorkflowSnapshot: async (req) => {
      for (const record of state.workflows) {
        const all = [...record.published, record.draft];
        const snapshot = all.find((item) => item.id === req.snapshotId);
        if (snapshot !== undefined) return { snapshot };
      }
      throw new Error(`snapshot ${req.snapshotId} not found`);
    },
  } satisfies TestHandlers;
}
