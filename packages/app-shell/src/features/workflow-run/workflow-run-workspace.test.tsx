import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createChatStore } from "@ora/chat";
import { PlatformProvider } from "../../platform";
import { createMemoryWorkflowRuntime } from "@ora/workflow-runtime/memory";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { createStubPlatform } from "../../test/stub-platform";
import { useLocationActionsStore } from "../../state/stores/location-actions-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { WorkflowRunWorkspace } from "./workflow-run-workspace";

vi.mock("../diff/task-diff-view", () => ({
  TaskDiffView: ({ toolbar }: { toolbar?: ReactNode }) => (
    <section aria-label="Task diff">
      <header data-diff-toolbar>{toolbar}</header>
    </section>
  ),
}));

vi.mock("../files/workspace-review-files-panel", () => ({
  WorkspaceReviewFilesPanel: ({ toolbar }: { toolbar?: ReactNode }) => (
    <section aria-label="Files panel">{toolbar}</section>
  ),
}));

const GRAPH = JSON.stringify({
  nodes: [
    {
      id: "start",
      type: "workflow",
      position: { x: 0, y: 0 },
      data: { kind: "start", title: "开始", description: "" },
    },
  ],
  edges: [],
  viewport: { x: 32, y: 32, zoom: 1 },
  description: "",
});

/** Seeds project + workflow + run-task so the workspace can load Changes and location actions. */
function seedRunWithTask() {
  const state = createMockClientState();
  state.projects = [{ id: "p1", name: "Demo", rootPath: "/demo" }];
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
  state.workflowRuns = [
    {
      id: "run-1",
      projectId: "p1",
      workflowId: "workflow-a",
      snapshotId: "snap-1",
      name: "审查流程 1",
      status: "pending",
      taskId: "task-run-1",
      createdAt: 1n,
      updatedAt: 1n,
    },
  ];
  state.tasks = [
    {
      id: "task-run-1",
      projectId: "p1",
      title: "审查流程 1",
      status: "todo",
      workspaceMode: "worktree",
      type: "workflow",
      workflowRunId: "run-1",
    },
  ];
  return state;
}

describe("WorkflowRunWorkspace", () => {
  beforeEach(() => {
    useWorkspaceSelectionStore.setState({
      selection: {
        projectId: "p1",
        taskId: null,
        sessionId: null,
        workflowRunId: "run-1",
      },
    });
    useLocationActionsStore.setState({ defaultTarget: "explorer" });
  });

  it("exposes the run-task Changes panel once the worktree task id is loaded", async () => {
    const state = seedRunWithTask();
    const client = createMockClient(state);
    const runtime = createMemoryWorkflowRuntime();
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
      runtime,
    );
    const user = userEvent.setup();

    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <Wrapper>
          <WorkflowRunWorkspace runId="run-1" />
        </Wrapper>
      </PlatformProvider>,
    );

    await waitFor(() => {
      expect(screen.getByText("审查流程 1")).toBeInTheDocument();
    });
    expect(
      screen.getByRole("button", { name: /变更|Changes/ }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /变更|Changes/ }));
    expect(
      screen.getByRole("region", { name: "Task diff" }),
    ).toBeInTheDocument();

    runtime.dispose();
  });

  it("exposes Desktop open-location actions against the run-task worktree", async () => {
    const state = seedRunWithTask();
    const client = createMockClient(state);
    const runtime = createMemoryWorkflowRuntime();
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
      runtime,
    );
    const user = userEvent.setup();
    const resolveTaskCwd = vi.fn(async () => "/demo/.ora/task-run-1");
    const open = vi.fn(async () => undefined);
    const platform = {
      ...createStubPlatform(),
      locationActions: {
        resolveTaskCwd,
        open,
      },
    };

    render(
      <PlatformProvider adapter={platform}>
        <Wrapper>
          <WorkflowRunWorkspace runId="run-1" />
        </Wrapper>
      </PlatformProvider>,
    );

    await waitFor(() => {
      expect(screen.getByText("审查流程 1")).toBeInTheDocument();
    });

    expect(
      screen.getByRole("group", { name: /打开位置|Open location/ }),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", {
        name: /用 文件管理器 打开|Open in File Manager/,
      }),
    );

    await waitFor(() => {
      expect(resolveTaskCwd).toHaveBeenCalledWith("task-run-1");
    });
    expect(open).toHaveBeenCalledWith("explorer", "/demo/.ora/task-run-1");

    runtime.dispose();
  });

  it("keeps the workspace project selected when restarting a run", async () => {
    const state = seedRunWithTask();
    // "Run again" is only offered on terminal runs.
    state.workflowRuns[0].status = "cancelled";
    const client = createMockClient(state);
    const runtime = createMemoryWorkflowRuntime();
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
      runtime,
    );
    const user = userEvent.setup();

    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <Wrapper>
          <WorkflowRunWorkspace runId="run-1" />
        </Wrapper>
      </PlatformProvider>,
    );

    await waitFor(() => {
      expect(screen.getByText("审查流程 1")).toBeInTheDocument();
    });

    await user.click(
      screen.getByRole("button", { name: /再次运行|Run again/ }),
    );

    // Regression: the display run stubs projectId as "", and re-selecting with it
    // would poison the workspace selection, making the next warm target an empty
    // project root. Restart must keep the real project id.
    await waitFor(() => {
      expect(useWorkspaceSelectionStore.getState().selection.projectId).toBe(
        "p1",
      );
    });

    runtime.dispose();
  });
});
