import { act } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import {
  createTestQueryClient,
  renderHookWithClient,
} from "../../test/hook-harness";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { queryKeys } from "./query-keys";
import { useCreateTask, useRenameSession } from "./use-workspace-mutations";

beforeEach(() => {
  useWorkspaceSelectionStore.getState().selectProject("p1");
});

describe("useRenameSession", () => {
  it("persists the new title onto the mock session", async () => {
    const state = createMockClientState();
    state.sessions = [
      {
        id: "s1",
        taskId: "t1",
        agentCli: "open_code",
        status: "running",
        title: "Old",
        historyState: { type: "writable" },
      },
    ];
    const client = createMockClient(state);
    const { result } = renderHookWithClient(
      () => useRenameSession(),
      client,
      createTestQueryClient(),
    );

    await act(async () => {
      await result.current.mutateAsync({ sessionId: "s1", title: "New title" });
    });

    expect(state.sessions[0]?.title).toBe("New title");
  });
});

describe("useCreateTask", () => {
  it.each([
    ["worktree", "worktree"],
    ["project_root", "project_root"],
  ] as const)(
    "forwards the %s workspace mode",
    async (_label, workspaceMode) => {
      const state = createMockClientState();
      const client = createMockClient(state);
      const { result } = renderHookWithClient(
        () => useCreateTask(),
        client,
        createTestQueryClient(),
      );

      await act(async () => {
        await result.current.mutateAsync({
          projectId: "p1",
          title: "Task",
          workspaceMode,
        });
      });

      expect(state.tasks[0]?.workspaceMode).toBe(workspaceMode);
      expect(useWorkspaceSelectionStore.getState().selection).toEqual(
        workspaceMode === "worktree"
          ? {
              projectId: "p1",
              taskId: "t1",
              sessionId: null,
              workflowRunId: null,
            }
          : {
              projectId: "p1",
              taskId: null,
              sessionId: null,
              workflowRunId: null,
            },
      );
    },
  );

  it("invalidates project branches after creating a worktree", async () => {
    const state = createMockClientState();
    const client = createMockClient(state);
    const queryClient = createTestQueryClient();
    const projectBranchesKey = queryKeys.projectBranches("p1");
    queryClient.setQueryData(projectBranchesKey, []);
    const { result } = renderHookWithClient(
      () => useCreateTask(),
      client,
      queryClient,
    );

    await act(async () => {
      await result.current.mutateAsync({
        projectId: "p1",
        title: "Task",
        workspaceMode: "worktree",
        baseBranch: "main",
      });
    });

    expect(queryClient.getQueryState(projectBranchesKey)?.isInvalidated).toBe(
      true,
    );
  });
});
