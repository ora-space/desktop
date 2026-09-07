import { type WorkflowRun } from "@ora/contracts";
import type { TestHandlers } from "../contracts-transport";
import { nextId, nextTimestamp } from "./records";
import { visibleWorkspaces, type WorkspaceMemoryState } from "./workspaces";

/** Mutable records owned by the workflow-runs test adapter. */
export interface WorkflowRunMemoryState {
  workflowRuns: MockWorkflowRunRecord[];
}

/** One in-memory workflow run used by run-hook tests. */
export interface MockWorkflowRunRecord {
  id: string;
  projectId: string;
  workflowId: string;
  snapshotId: string;
  name: string;
  status: "pending" | "running" | "succeeded" | "failed" | "cancelled";
  workspaceId: string;
  createdAt: bigint;
  updatedAt: bigint;
}

/** Creates an independent workflow-runs memory fixture. */
export function createWorkflowRunMemory(): WorkflowRunMemoryState {
  return { workflowRuns: [] };
}

/** Builds the public run payload for one mock workflow-run record. */
function mockWorkflowRun(record: MockWorkflowRunRecord): WorkflowRun {
  return {
    id: record.id,
    workspaceId: record.workspaceId,
    workflowId: record.workflowId,
    snapshotId: record.snapshotId,
    name: record.name,
    status: record.status,
    state: '{"current_nodes":[]}',
    input: null,
    output: null,
    error: null,
    startedAt: null,
    finishedAt: null,
    createdAt: record.createdAt,
    updatedAt: record.updatedAt,
  };
}

/** Registers only the workflow-runs operations explicitly requested by a fixture. */
export function workflowRunHandlers(
  state: WorkflowRunMemoryState & WorkspaceMemoryState,
) {
  return {
    createWorkflowRun: async (req) => {
      const id = nextId("run", state.workflowRuns.length);
      const now = nextTimestamp();
      const run: WorkflowRun = {
        id,
        workspaceId: req.workspaceId,
        workflowId: req.workflowId,
        snapshotId: "snap-1",
        name: req.name ?? "",
        status: "pending",
        state: '{"current_nodes":[]}',
        input: null,
        output: null,
        error: null,
        startedAt: null,
        finishedAt: null,
        createdAt: now,
        updatedAt: now,
      };
      state.workflowRuns.push({
        id,
        projectId:
          visibleWorkspaces(state).find(
            (workspace) => workspace.id === req.workspaceId,
          )?.projectId ?? "",
        workspaceId: req.workspaceId,
        workflowId: req.workflowId,
        snapshotId: run.snapshotId,
        name: req.name ?? "",
        status: "pending",
        createdAt: now,
        updatedAt: now,
      });
      return { run };
    },
    getWorkflowRun: async (req) => {
      const record = state.workflowRuns.find(
        (candidate) => candidate.id === req.runId,
      );
      if (record === undefined)
        throw new Error(`workflow run ${req.runId} not found`);
      return {
        run: {
          id: record.id,
          workspaceId: record.workspaceId,
          workflowId: record.workflowId,
          snapshotId: record.snapshotId,
          name: record.name,
          status: record.status,
          state: '{"current_nodes":[]}',
          input: null,
          output: null,
          error: null,
          startedAt: null,
          finishedAt: null,
          createdAt: record.createdAt,
          updatedAt: record.updatedAt,
        },
        name: record.name,
        projectId: record.projectId,
        workspaceId: record.workspaceId,
        variables: [],
        conditionDecisions: {},
        nodes: [],
      };
    },
    startWorkflowRun: async (req) => {
      const record = state.workflowRuns.find(
        (candidate) => candidate.id === req.runId,
      );
      if (record === undefined)
        throw new Error(`workflow run ${req.runId} not found`);
      return { run: mockWorkflowRun(record) };
    },
    cancelWorkflowRun: async (req) => {
      const record = state.workflowRuns.find(
        (candidate) => candidate.id === req.runId,
      );
      if (record === undefined)
        throw new Error(`workflow run ${req.runId} not found`);
      return { run: mockWorkflowRun(record) };
    },
    restartWorkflowRun: async (req) => {
      const record = state.workflowRuns.find(
        (candidate) => candidate.id === req.runId,
      );
      if (record === undefined)
        throw new Error(`workflow run ${req.runId} not found`);
      return { run: mockWorkflowRun(record) };
    },
    updateWorkflowRunInput: async (req) => {
      const record = state.workflowRuns.find(
        (candidate) => candidate.id === req.runId,
      );
      if (record === undefined)
        throw new Error(`workflow run ${req.runId} not found`);
      return { run: mockWorkflowRun(record) };
    },
    completeWorkflowNode: async (req) => {
      const record = state.workflowRuns.find(
        (candidate) => candidate.id === req.runId,
      );
      if (record === undefined)
        throw new Error(`workflow run ${req.runId} not found`);
      record.status = "succeeded";
      return { run: mockWorkflowRun(record) };
    },
    listWorkflowRuns: async (req) => ({
      runs: state.workflowRuns
        .filter((record) => record.projectId === req.projectId)
        .map((record) => ({
          id: record.id,
          name: record.name,
          workspaceId: record.workspaceId,
          projectId: record.projectId,
          workflowId: record.workflowId,
          status: record.status,
          startedAt: null,
          finishedAt: null,
          createdAt: record.createdAt,
        })),
    }),
    listWorkflowRunsByWorkflow: async (req) => ({
      runs: state.workflowRuns
        .filter((record) => record.workflowId === req.workflowId)
        .map((record) => ({
          id: record.id,
          name: record.name,
          workspaceId: record.workspaceId,
          projectId: record.projectId,
          workflowId: record.workflowId,
          status: record.status,
          startedAt: null,
          finishedAt: null,
          createdAt: record.createdAt,
        })),
    }),
    listWorkflowNodeRuns: async () => ({ nodes: [] }),
    deleteWorkflowRun: async (req) => {
      const idx = state.workflowRuns.findIndex(
        (record) => record.id === req.runId,
      );
      if (idx >= 0) state.workflowRuns.splice(idx, 1);
      return { runId: req.runId };
    },
    renameWorkflowRun: async (req) => {
      const record = state.workflowRuns.find(
        (candidate) => candidate.id === req.runId,
      );
      if (record === undefined)
        throw new Error(`workflow run ${req.runId} not found`);
      record.name = req.name;
      return { run: mockWorkflowRun(record) };
    },
  } satisfies TestHandlers;
}
