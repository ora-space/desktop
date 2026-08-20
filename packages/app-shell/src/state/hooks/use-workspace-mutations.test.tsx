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
import { useDraftSessionsStore } from "../stores/draft-sessions-store";
import { useComposerInputStore } from "../stores/composer-input-store";
import { queryKeys } from "./query-keys";
import {
  useCreateTask,
  useDeleteProject,
  useDeleteSession,
  useDeleteTask,
  useRenameSession,
} from "./use-workspace-mutations";

beforeEach(() => {
  useDraftSessionsStore.getState().clear();
  useComposerInputStore.getState().reset();
  useWorkspaceSelectionStore.getState().selectProject("p1");
});

describe("useRenameSession", () => {
  it("persists the new title onto the mock session", async () => {
    const state = createMockClientState();
    state.sessions = [
      {
        id: "s1",
        taskId: "t1",
        agentRef: "ora-space.opencode",
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

describe("delete mutations clear parked composer state", () => {
  it("clears composer input and bound drafts when a session is deleted", async () => {
    const state = createMockClientState();
    state.sessions = [
      {
        id: "s1",
        taskId: "t1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const client = createMockClient(state);
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(queryKeys.sessions, state.sessions);
    useComposerInputStore.getState().setInput("s1", {
      text: "parked",
      images: [],
    });
    const draftId = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: "t1" });
    useDraftSessionsStore.getState().updateContent(draftId, { text: "bound" });
    useDraftSessionsStore.getState().bindToSession(draftId, "s1");

    const { result } = renderHookWithClient(
      () => useDeleteSession(),
      client,
      queryClient,
    );
    await act(async () => {
      await result.current.mutateAsync({ sessionId: "s1" });
    });

    expect(useComposerInputStore.getState().byKey.s1).toBeUndefined();
    expect(useDraftSessionsStore.getState().drafts).toHaveLength(0);
  });

  it("scrubs returnTo pointing at a deleted session", async () => {
    const state = createMockClientState();
    state.sessions = [
      {
        id: "s1",
        taskId: "t1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const client = createMockClient(state);
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(queryKeys.sessions, state.sessions);
    const draftId = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: "t2" });
    useDraftSessionsStore.getState().updateContent(draftId, { text: "parked" });
    useDraftSessionsStore.getState().setReturnTo(draftId, {
      sessionId: "s1",
      taskId: "t1",
      projectId: "p1",
    });

    const { result } = renderHookWithClient(
      () => useDeleteSession(),
      client,
      queryClient,
    );
    await act(async () => {
      await result.current.mutateAsync({ sessionId: "s1" });
    });

    expect(
      useDraftSessionsStore.getState().drafts.find((d) => d.id === draftId)
        ?.returnTo,
    ).toBeNull();
  });

  it("clears drafts and session parks when a task is deleted", async () => {
    const state = createMockClientState();
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        title: "Task",
        workspaceMode: "worktree",
        type: "default",
        workflowRunId: null,
      },
    ];
    state.sessions = [
      {
        id: "s1",
        taskId: "t1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const client = createMockClient(state);
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(queryKeys.tasks, state.tasks);
    queryClient.setQueryData(queryKeys.sessions, state.sessions);
    useComposerInputStore.getState().setInput("s1", {
      text: "parked",
      images: [],
    });
    useComposerInputStore.getState().setInput("task:t1", {
      text: "task park",
      images: [],
    });
    const draftId = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: "t1" });
    useDraftSessionsStore.getState().updateContent(draftId, { text: "keep?" });

    const { result } = renderHookWithClient(
      () => useDeleteTask(),
      client,
      queryClient,
    );
    await act(async () => {
      await result.current.mutateAsync({ taskId: "t1" });
    });

    expect(useComposerInputStore.getState().byKey.s1).toBeUndefined();
    expect(useComposerInputStore.getState().byKey["task:t1"]).toBeUndefined();
    expect(useDraftSessionsStore.getState().drafts).toHaveLength(0);
  });

  it("clears project drafts and related session parks when a project is deleted", async () => {
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora", rootPath: "/ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        title: "Task",
        workspaceMode: "worktree",
        type: "default",
        workflowRunId: null,
      },
    ];
    state.sessions = [
      {
        id: "s1",
        taskId: "t1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const client = createMockClient(state);
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(queryKeys.projects, state.projects);
    queryClient.setQueryData(queryKeys.tasks, state.tasks);
    queryClient.setQueryData(queryKeys.sessions, state.sessions);
    useComposerInputStore.getState().setInput("s1", {
      text: "parked",
      images: [],
    });
    useComposerInputStore.getState().setInput("task:t1", {
      text: "task park",
      images: [],
    });
    const draftId = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().updateContent(draftId, { text: "direct" });

    const { result } = renderHookWithClient(
      () => useDeleteProject(),
      client,
      queryClient,
    );
    await act(async () => {
      await result.current.mutateAsync({ projectId: "p1" });
    });

    expect(useComposerInputStore.getState().byKey.s1).toBeUndefined();
    expect(useComposerInputStore.getState().byKey["task:t1"]).toBeUndefined();
    expect(useDraftSessionsStore.getState().drafts).toHaveLength(0);
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
              draftId: expect.any(String),
            }
          : {
              projectId: "p1",
              taskId: null,
              sessionId: null,
              workflowRunId: null,
              draftId: null,
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
