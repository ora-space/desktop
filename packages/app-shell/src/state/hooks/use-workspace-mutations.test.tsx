import { act } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createWorkspaceMemory,
  workspaceHandlers,
} from "../../test/memory/workspaces";
import {
  createSessionMemory,
  sessionHandlers,
} from "../../test/memory/sessions";
import "../../i18n/i18n-instance";
import {
  createTestQueryClient,
  renderHookWithClient,
} from "../../test/hook-harness";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { useDraftSessionsStore } from "../stores/draft-sessions-store";
import { useComposerInputStore } from "../stores/composer-input-store";
import { sessionKeys } from "../data/sessions";
import { workspaceKeys } from "../data/workspace";
import {
  useCreateTask,
  useDeleteProject,
  useDeleteSession,
  useDeleteTask,
  useRenameSession,
} from "./use-workspace-mutations";
import { AGENT_REF } from "../../test/agent-identity";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return { ...createWorkspaceMemory(), ...createSessionMemory() };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...workspaceHandlers(state),
    ...sessionHandlers(state),
  };
}

beforeEach(() => {
  useDraftSessionsStore.getState().clear();
  useComposerInputStore.getState().reset();
  useWorkspaceSelectionStore.getState().selectProject("p1");
});

describe("useRenameSession", () => {
  it("persists the new title onto the mock session", async () => {
    const state = createFixtureState();
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: "Old",
        historyState: { type: "writable" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(sessionKeys.sessions, state.sessions);
    const { result } = renderHookWithClient(
      () => useRenameSession(),
      client,
      queryClient,
    );

    await act(async () => {
      await result.current.mutateAsync({ sessionId: "s1", title: "New title" });
    });

    expect(state.sessions[0]?.title).toBe("New title");
    expect(queryClient.getQueryData(sessionKeys.sessions)).toEqual([
      expect.objectContaining({ id: "s1", title: "New title" }),
    ]);
    // Patch-only: do not force an active list refetch that rebuilds every row.
    expect(queryClient.isFetching({ queryKey: sessionKeys.sessions })).toBe(0);
  });
});

describe("delete mutations clear parked composer state", () => {
  it("optimistically removes a session from the list cache", async () => {
    const state = createFixtureState();
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
      {
        id: "s2",
        workspaceId: "workspace-t1",
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(sessionKeys.sessions, state.sessions);
    const { result } = renderHookWithClient(
      () => useDeleteSession(),
      client,
      queryClient,
    );
    await act(async () => {
      await result.current.mutateAsync({ sessionId: "s1", listSync: "defer" });
    });
    expect(queryClient.getQueryData(sessionKeys.sessions)).toEqual([
      expect.objectContaining({ id: "s2" }),
    ]);
    expect(queryClient.isFetching({ queryKey: sessionKeys.sessions })).toBe(0);
  });

  it("clears composer input and bound drafts when a session is deleted", async () => {
    const state = createFixtureState();
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(sessionKeys.sessions, state.sessions);
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
    const state = createFixtureState();
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(sessionKeys.sessions, state.sessions);
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
    const state = createFixtureState();
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Task",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(workspaceKeys.tasks, state.tasks);
    queryClient.setQueryData(sessionKeys.sessions, state.sessions);
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
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Task",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(workspaceKeys.projects, state.projects);
    queryClient.setQueryData(workspaceKeys.tasks, state.tasks);
    queryClient.setQueryData(sessionKeys.sessions, state.sessions);
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
  it("creates a worktree task and selects its workspace draft", async () => {
    const state = createFixtureState();
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const { result } = renderHookWithClient(
      () => useCreateTask(),
      client,
      createTestQueryClient(),
    );

    await act(async () => {
      await result.current.mutateAsync({
        projectId: "p1",
        title: "Task",
      });
    });

    expect(state.tasks[0]?.workspaceId).toBe("workspace-t1");
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: "t1",
      sessionId: null,
      workflowRunId: null,
      draftId: expect.any(String),
    });
  });

  it("invalidates project branches after creating a worktree", async () => {
    const state = createFixtureState();
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const queryClient = createTestQueryClient();
    const projectBranchesKey = workspaceKeys.projectBranches("p1");
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
        baseBranch: "main",
      });
    });

    expect(queryClient.getQueryState(projectBranchesKey)?.isInvalidated).toBe(
      true,
    );
  });
});
