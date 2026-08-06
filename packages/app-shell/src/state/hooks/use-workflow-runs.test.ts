import { describe, expect, it, vi } from "vitest";
import { renderHookWithClient } from "../../test/hook-harness";
import { createMockClient, createMockClientState, type MockClientState } from "../../test/mock-client";
import { buildDisplayRun, useDeleteWorkflowRun, useRenameWorkflowRun, useWorkflowRunsByProject } from "./use-workflow-runs";

/** Seeds one persisted run and its run-task for hook tests. */
function seededState(): MockClientState {
  const state = createMockClientState();
  state.workflowRuns = [{
    id: "run-1",
    projectId: "p1",
    workflowId: "workflow-a",
    snapshotId: "snap-1",
    name: "审查流程 1",
    status: "pending",
    taskId: "t1",
    createdAt: 1n,
    updatedAt: 1n,
  }];
  state.tasks = [{
    id: "t1",
    projectId: "p1",
    title: "审查流程 1",
    status: "todo",
    workspaceMode: "worktree",
    type: "workflow",
    workflowRunId: "run-1",
  }];
  return state;
}

const GRAPH = JSON.stringify({
  nodes: [
    {
      id: "start",
      type: "workflow",
      position: { x: 0, y: 0 },
      data: { kind: "start", title: "开始", description: "" },
    },
    {
      id: "explore",
      type: "workflow",
      position: { x: 200, y: 0 },
      data: { kind: "agent", title: "探索", description: "" },
    },
  ],
  edges: [{ id: "e1", source: "start", target: "explore" }],
  viewport: { x: 32, y: 32, zoom: 1 },
  description: "审查流程",
});

describe("buildDisplayRun", () => {
  const detail = {
    run: {
      id: "run-1",
      workflowId: "workflow-a",
      status: "pending",
      state: "{\"current_nodes\":[\"prompt-1\"]}",
      startedAt: null,
      finishedAt: null,
      createdAt: 1n,
      updatedAt: 1n,
    },
    name: "审查流程 1",
    nodes: [
      {
        nodeId: "explore",
        status: "running",
        startedAt: 2n,
        finishedAt: null,
        error: null,
        output: null,
      },
    ],
  };

  it("projects a paused pending run to awaiting_input", () => {
    const display = buildDisplayRun(detail, GRAPH);
    expect(display.status).toBe("awaiting_input");
  });

  it("builds the definition snapshot and per-node states from the frozen graph", () => {
    const display = buildDisplayRun(detail, GRAPH);
    expect(display.definitionSnapshot.name).toBe("审查流程 1");
    expect(display.definitionSnapshot.description).toBe("审查流程");
    expect(display.nodeStates.start).toEqual({ status: "idle" });
    expect(display.nodeStates.explore.status).toBe("running");
    expect(display.nodeStates.explore.startedAt).toBe(new Date(2).toISOString());
  });

  it("derives awaiting_input node state from a pending node-run", () => {
    const pendingDetail = {
      ...detail,
      nodes: [{ nodeId: "explore", status: "pending", startedAt: null, finishedAt: null, error: null, output: null }],
    };
    const display = buildDisplayRun(pendingDetail, GRAPH);
    expect(display.nodeStates.explore.status).toBe("awaiting_input");
  });
});

describe("persisted run hooks", () => {
  it("lists the persisted runs of a project", async () => {
    const state = seededState();
    const client = createMockClient(state);
    const { result } = renderHookWithClient(() => useWorkflowRunsByProject("p1"), client);
    await vi.waitFor(() => expect(result.current.data).toBeDefined());
    expect(result.current.data).toEqual([{
      id: "run-1",
      name: "审查流程 1",
      projectId: "p1",
      workflowId: "workflow-a",
      status: "pending",
      startedAt: null,
      finishedAt: null,
      createdAt: 1n,
    }]);
  });

  it("renames a run through its run-task title", async () => {
    const state = seededState();
    const client = createMockClient(state);
    const { result } = renderHookWithClient(() => useRenameWorkflowRun(), client);
    await result.current.mutateAsync({ runId: "run-1", name: "审查流程 v2" });
    expect(state.tasks.find((task) => task.id === "t1")?.title).toBe("审查流程 v2");
  });

  it("deletes a run and refreshes its project list", async () => {
    const state = seededState();
    const client = createMockClient(state);
    const { result } = renderHookWithClient(() => useDeleteWorkflowRun(), client);
    await result.current.mutateAsync({ runId: "run-1", projectId: "p1" });
    expect(state.workflowRuns).toEqual([]);
  });
});
