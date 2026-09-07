import { QueryClient } from "@tanstack/react-query";
import { mockWorkflowKeys } from "./mock-workflows";
import { waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { renderHookWithClient } from "../../test/hook-harness";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { createWorkspaceMemory } from "../../test/memory/workspaces";
import {
  createWorkflowMemory,
  workflowHandlers,
} from "../../test/memory/workflows";
import {
  createWorkflowRunMemory,
  workflowRunHandlers,
} from "../../test/memory/workflow-runs";
import "../../i18n/i18n-instance";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import {
  buildDisplayRun,
  workflowRunKeys,
  useDeleteWorkflowRun,
  useRealWorkflowRun,
  useRenameWorkflowRun,
  useWorkflowRunsByProject,
} from "./workflow-runs";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return {
    ...createWorkspaceMemory(),
    ...createWorkflowMemory(),
    ...createWorkflowRunMemory(),
  };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...workflowHandlers(state),
    ...workflowRunHandlers(state),
  };
}

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
});

/** Seeds one persisted run and its run-task for hook tests. */
function seededState(): FixtureState {
  const state = createFixtureState();
  state.workflowRuns = [
    {
      id: "run-1",
      projectId: "p1",
      workflowId: "workflow-a",
      snapshotId: "snap-1",
      name: "审查流程 1",
      status: "pending",
      workspaceId: "workspace-t1",
      createdAt: 1n,
      updatedAt: 1n,
    },
  ];
  state.tasks = [
    {
      id: "t1",
      projectId: "p1",
      workspaceId: "workspace-t1",
      title: "审查流程 1",
    },
  ];
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
      workspaceId: "workspace-t1",
      workflowId: "workflow-a",
      snapshotId: "snap-1",
      name: "审查流程 1",
      status: "pending",
      state: '{"current_nodes":["prompt-1"]}',
      input: null,
      startedAt: null,
      finishedAt: null,
      createdAt: 1n,
      updatedAt: 1n,
    },
    name: "审查流程 1",
    projectId: "p1",
    variables: [],
    conditionDecisions: {},
    nodes: [
      {
        nodeId: "explore",
        status: "running",
        startedAt: 2n,
        finishedAt: null,
        error: null,
        output: null,
        payload: null,
      },
    ],
  };

  it("projects a paused pending run to awaiting_input", () => {
    const display = buildDisplayRun(detail, GRAPH);
    expect(display.status).toBe("awaiting_input");
  });

  it("carries the run-task's real project id onto the display run", () => {
    const display = buildDisplayRun(detail, GRAPH);
    expect(display.projectId).toBe("p1");
  });

  it("builds the definition snapshot and per-node states from the frozen graph", () => {
    const display = buildDisplayRun(detail, GRAPH);
    expect(display.definitionSnapshot.name).toBe("审查流程 1");
    expect(display.definitionSnapshot.description).toBe("审查流程");
    expect(display.nodeStates.start).toEqual({ status: "idle" });
    expect(display.nodeStates.explore.status).toBe("running");
    expect(display.nodeStates.explore.startedAt).toBe(
      new Date(2).toISOString(),
    );
  });

  it("projects node session ids for future stage-scoped Diff", () => {
    const withSession = {
      ...detail,
      nodes: [
        {
          nodeId: "explore",
          status: "running",
          startedAt: 2n,
          finishedAt: null,
          error: null,
          output: null,
          payload: null,
          sessionId: "session-explore",
        },
      ],
    };
    const display = buildDisplayRun(withSession, GRAPH);
    expect(display.nodeStates.explore.sessionId).toBe("session-explore");
  });

  it("surfaces the committed run input on the start node as kickoff input", () => {
    const display = buildDisplayRun(
      {
        ...detail,
        run: { ...detail.run, input: "只审查 README" },
      },
      GRAPH,
    );
    expect(display.kickoffInput).toBe("只审查 README");
    const startNode = display.definitionSnapshot.nodes.find(
      (node) => node.id === "start",
    );
    expect(startNode?.data.input).toBe("只审查 README");
  });

  it("projects assigned and unassigned typed variables onto the Start node", () => {
    const graph = JSON.stringify({
      ...JSON.parse(GRAPH),
      nodes: [
        {
          id: "start",
          type: "workflow",
          position: { x: 0, y: 0 },
          data: {
            kind: "start",
            title: "开始",
            description: "",
            inputVariables: [
              { name: "count", valueType: "integer", value: 1 },
              { name: "tags", valueType: "array", value: ["draft"] },
            ],
          },
        },
        JSON.parse(GRAPH).nodes[1],
      ],
    });
    const display = buildDisplayRun(
      {
        ...detail,
        variables: [
          { selector: ["start", "count"], value: 3 },
          { selector: ["start", "tags"] },
        ],
      },
      graph,
    );
    const startNode = display.definitionSnapshot.nodes.find(
      (node) => node.id === "start",
    );

    expect(startNode?.data.inputVariables).toEqual([
      { name: "count", valueType: "integer", value: 3 },
      { name: "tags", valueType: "array", value: undefined },
    ]);
  });

  it("projects node file changes from the run payload", () => {
    const withFiles = {
      ...detail,
      nodes: [
        {
          nodeId: "explore",
          status: "succeeded",
          startedAt: 10n,
          finishedAt: 30n,
          error: null,
          output: null,
          payload:
            '{"file_changes":[{"path":"src/a.ts","additions":1,"deletions":0},{"path":"src/new.ts","additions":1,"deletions":0}]}',
        },
      ],
    };
    const display = buildDisplayRun(withFiles, GRAPH);
    expect(display.nodeStates.explore.fileChanges).toEqual([
      { path: "src/a.ts", additions: 1, deletions: 0 },
      { path: "src/new.ts", additions: 1, deletions: 0 },
    ]);
  });

  it("projects the node conversation from its run output", () => {
    const withConversation = {
      ...detail,
      nodes: [
        {
          nodeId: "explore",
          status: "succeeded",
          startedAt: 10n,
          finishedAt: 30n,
          error: null,
          output:
            '[{"role":"user","text":"帮我审查"},{"role":"assistant","text":"好的，开始"}]',
          payload: null,
        },
      ],
    };
    const display = buildDisplayRun(withConversation, GRAPH);
    expect(display.nodeStates.explore.conversation).toEqual([
      {
        kind: "message",
        id: "node-output-0",
        runId: "run-1",
        nodeId: "explore",
        sessionId: "",
        role: "user",
        markdown: "帮我审查",
        status: "complete",
        createdAt: new Date(10).toISOString(),
        updatedAt: new Date(10).toISOString(),
      },
      {
        kind: "message",
        id: "node-output-1",
        runId: "run-1",
        nodeId: "explore",
        sessionId: "",
        role: "assistant",
        markdown: "好的，开始",
        status: "complete",
        createdAt: new Date(1010).toISOString(),
        updatedAt: new Date(1010).toISOString(),
      },
    ]);
  });

  it("keeps Output JSON as output instead of projecting an Agent conversation", () => {
    const outputGraph = JSON.stringify({
      nodes: [
        ...JSON.parse(GRAPH).nodes,
        {
          id: "output-1",
          type: "workflow",
          position: { x: 400, y: 0 },
          data: { kind: "output", title: "输出", description: "" },
        },
      ],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
    });
    const output = '[{"role":"assistant","text":"这也是合法的业务结果"}]';
    const display = buildDisplayRun(
      {
        ...detail,
        nodes: [
          {
            nodeId: "output-1",
            status: "succeeded",
            startedAt: 10n,
            finishedAt: 20n,
            error: null,
            output,
            payload: null,
          },
        ],
      },
      outputGraph,
    );

    expect(display.nodeStates["output-1"]).toEqual({
      status: "succeeded",
      startedAt: new Date(10).toISOString(),
      finishedAt: new Date(20).toISOString(),
      output: { summary: output },
    });
  });

  it("derives awaiting_input node state from a pending node-run", () => {
    const pendingDetail = {
      ...detail,
      nodes: [
        {
          nodeId: "explore",
          status: "pending",
          startedAt: null,
          finishedAt: null,
          error: null,
          output: null,
          payload: null,
        },
      ],
    };
    const display = buildDisplayRun(pendingDetail, GRAPH);
    expect(display.nodeStates.explore.status).toBe("awaiting_input");
  });

  it("marks nodes behind a lost condition branch inactive from the run variable pool", () => {
    const branching = JSON.stringify({
      nodes: [
        {
          id: "start",
          type: "workflow",
          position: { x: 0, y: 0 },
          data: { kind: "start", title: "", description: "" },
        },
        {
          id: "c",
          type: "workflow",
          position: { x: 100, y: 0 },
          data: { kind: "condition", title: "", description: "" },
        },
        {
          id: "ok",
          type: "workflow",
          position: { x: 200, y: 0 },
          data: { kind: "agent", title: "", description: "" },
        },
        {
          id: "no",
          type: "workflow",
          position: { x: 200, y: 100 },
          data: { kind: "agent", title: "", description: "" },
        },
      ],
      edges: [
        { id: "e1", source: "start", target: "c" },
        { id: "e2", source: "c", sourceHandle: "approved", target: "ok" },
        { id: "e3", source: "c", sourceHandle: "else", target: "no" },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    });
    const branchingDetail = {
      ...detail,
      run: {
        ...detail.run,
        status: "succeeded",
      },
      conditionDecisions: { c: "approved" },
      nodes: [
        {
          nodeId: "start",
          status: "succeeded",
          startedAt: 1n,
          finishedAt: 2n,
          error: null,
          output: null,
          payload: null,
        },
        {
          nodeId: "c",
          status: "succeeded",
          startedAt: 1n,
          finishedAt: 2n,
          error: null,
          output: null,
          payload: null,
        },
      ],
    };
    const display = buildDisplayRun(branchingDetail, branching);
    expect(display.nodeStates.ok.status).toBe("idle");
    expect(display.nodeStates.no.status).toBe("inactive");
  });
});

describe("useRealWorkflowRun", () => {
  it("returns the display run together with its workspace id", async () => {
    const state = seededState();
    state.projects = [{ id: "p1", name: "Demo" }];
    state.workflows = [
      {
        workflow: {
          id: "workflow-a",
          namespace: "local",
          name: "审查流程",
          publishedSnapshotId: "snap-1",
          createdAt: 1n,
          updatedAt: 1n,
        },
        draft: {
          id: "draft-1",
          workflowId: "workflow-a",
          version: "draft",
          graph: GRAPH,
          createdAt: 1n,
          updatedAt: 1n,
        },
        published: [
          {
            id: "snap-1",
            workflowId: "workflow-a",
            version: "v1",
            graph: GRAPH,
            createdAt: 1n,
            updatedAt: null,
          },
        ],
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const { result } = renderHookWithClient(
      () => useRealWorkflowRun("run-1"),
      client,
    );
    await waitFor(() => expect(result.current.data).toBeDefined());
    expect(result.current.data?.workspaceId).toBe("workspace-t1");
    expect(result.current.data?.run.id).toBe("run-1");
    expect(result.current.data?.run.name).toBe("审查流程 1");
  });
});

describe("persisted run hooks", () => {
  it("lists the persisted runs of a project", async () => {
    const state = seededState();
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const { result } = renderHookWithClient(
      () => useWorkflowRunsByProject("p1"),
      client,
    );
    await waitFor(() => expect(result.current.data).toBeDefined());
    expect(result.current.data).toEqual([
      {
        id: "run-1",
        name: "审查流程 1",
        workspaceId: "workspace-t1",
        projectId: "p1",
        workflowId: "workflow-a",
        status: "pending",
        startedAt: null,
        finishedAt: null,
        createdAt: 1n,
      },
    ]);
  });

  it("renames a run through its workspace-owned name", async () => {
    const state = seededState();
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const { result } = renderHookWithClient(
      () => useRenameWorkflowRun(),
      client,
    );
    await result.current.mutateAsync({ runId: "run-1", name: "审查流程 v2" });
    expect(state.workflowRuns[0]?.name).toBe("审查流程 v2");
  });

  it("deletes a run and refreshes its project list", async () => {
    const state = seededState();
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const { result } = renderHookWithClient(
      () => useDeleteWorkflowRun(),
      client,
    );
    await result.current.mutateAsync({ runId: "run-1", projectId: "p1" });
    expect(state.workflowRuns).toEqual([]);
  });

  it("retires the selection when the deleted run is the one open in the workspace", async () => {
    const state = seededState();
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    useWorkspaceSelectionStore.getState().selectWorkflowRun("run-1", "p1");
    const { result } = renderHookWithClient(
      () => useDeleteWorkflowRun(),
      client,
    );
    await result.current.mutateAsync({ runId: "run-1", projectId: "p1" });
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: null,
      sessionId: null,
      workflowRunId: null,
      draftId: null,
    });
  });

  it("keeps the selection when a different run is deleted", async () => {
    const state = seededState();
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    useWorkspaceSelectionStore.getState().selectWorkflowRun("run-other", "p1");
    const { result } = renderHookWithClient(
      () => useDeleteWorkflowRun(),
      client,
    );
    await result.current.mutateAsync({ runId: "run-1", projectId: "p1" });
    expect(useWorkspaceSelectionStore.getState().selection.workflowRunId).toBe(
      "run-other",
    );
  });
});

it("keeps persisted and memory-runtime run identities and list prefixes separate", async () => {
  const client = new QueryClient();
  const persisted = workflowRunKeys.detail("run-1");
  const memory = mockWorkflowKeys.workflowRun("run-1");
  expect(persisted).toEqual(["workflowRun", "detail", "run-1"]);
  expect(memory).toEqual(["workflowRun", "run-1"]);
  client.setQueryData(persisted, { source: "persisted" });
  client.setQueryData(memory, { source: "memory" });
  client.setQueryData(workflowRunKeys.byProject("p1"), []);
  client.setQueryData(mockWorkflowKeys.workflowRuns("p1"), []);
  await client.invalidateQueries({ queryKey: workflowRunKeys.projectLists });
  expect(
    [
      persisted,
      memory,
      workflowRunKeys.byProject("p1"),
      mockWorkflowKeys.workflowRuns("p1"),
    ].map((key) => client.getQueryState(key)?.isInvalidated),
  ).toEqual([false, false, true, false]);
  client.removeQueries({ queryKey: persisted });
  expect(client.getQueryData(memory)).toEqual({ source: "memory" });
  client.clear();
});
