import { describe, it, expect, beforeEach } from "vitest";
import { waitFor } from "@testing-library/react";
import {
  useCreateProject,
  useUpdateProject,
  useDeleteProject,
  useCreateTask,
  useUpdateTask,
  useDeleteTask,
  useCreateSession,
  useUpdateSession,
  useDeleteSession,
} from "./use-workspace-mutations";
import { useProjects } from "./use-projects";
import { useTasks } from "./use-tasks";
import { useSessions } from "./use-sessions";
import { createMockClient, createMockClientState } from "../../test/mock-client";
import { renderHookWithClient } from "../../test/hook-harness";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { useUiStore } from "../stores/ui-store";
import { queryKeys } from "./query-keys";
import type { Project, Session, Task } from "@ora/contracts";
import { createChatStore, type AcpClient } from "@ora/chat";

const P1: Project = { id: "p1", name: "Ora", rootPath: "/ora" };
const P2: Project = { id: "p2", name: "Rustun", rootPath: "/rustun" };
const T1: Task = { id: "t1", projectId: "p1", title: "Refactor", status: "todo" };
const T2: Task = { id: "t2", projectId: "p1", title: "Docs", status: "doing" };
const S1: Session = { id: "s1", taskId: "t1", agentId: "codex", agentSessionId: null, status: "running" };
const S2: Session = { id: "s2", taskId: "t1", agentId: "claude", agentSessionId: null, status: "stopped" };

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
  useUiStore.setState({ expandedProjects: new Set(), expandedTasks: new Set() });
});

describe("useCreateProject", () => {
  it("creates a project, invalidates the list, and selects the new project", async () => {
    const state = createMockClientState();
    const client = createMockClient(state);
    const projects = renderHookWithClient(() => useProjects(), client);
    const mutation = renderHookWithClient(() => useCreateProject(), client, projects.queryClient);

    await waitFor(() => expect(projects.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({ name: "New", rootPath: "/new" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.projects).toHaveLength(1);
    expect(state.projects[0]!.name).toBe("New");

    // Selection switched to the new project.
    expect(useWorkspaceSelectionStore.getState().selection.projectId).toBe(state.projects[0]!.id);

    // The query cache was invalidated; refetch surfaces the new list.
    await projects.queryClient.refetchQueries({ queryKey: queryKeys.projects });
    expect(projects.queryClient.getQueryData<Project[]>(queryKeys.projects)).toHaveLength(1);
  });
});

describe("useUpdateProject", () => {
  it("updates the project and invalidates the list", async () => {
    const state = createMockClientState();
    state.projects = [P1];
    const client = createMockClient(state);
    const projects = renderHookWithClient(() => useProjects(), client);
    const mutation = renderHookWithClient(() => useUpdateProject(), client, projects.queryClient);

    await waitFor(() => expect(projects.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({ project: P1, name: "Renamed", rootPath: "/renamed" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.projects[0]).toEqual({ id: "p1", name: "Renamed", rootPath: "/renamed" });
  });
});

describe("useDeleteProject", () => {
  it("cascades child tasks and sessions before deleting the project", async () => {
    const state = createMockClientState();
    state.projects = [P1, P2];
    state.tasks = [T1, T2];
    state.sessions = [S1, S2];
    const client = createMockClient(state);

    const projects = renderHookWithClient(() => useProjects(), client);
    const tasks = renderHookWithClient(() => useTasks(), client, projects.queryClient);
    const sessions = renderHookWithClient(() => useSessions(), client, projects.queryClient);
    const mutation = renderHookWithClient(() => useDeleteProject(), client, projects.queryClient);

    await waitFor(() => expect(projects.result.current.isSuccess).toBe(true));
    await waitFor(() => expect(tasks.result.current.isSuccess).toBe(true));
    await waitFor(() => expect(sessions.result.current.isSuccess).toBe(true));

    mutation.result.current.mutate({ projectId: "p1" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.projects.map((p) => p.id)).toEqual(["p2"]);
    expect(state.tasks).toEqual([]);
    expect(state.sessions).toEqual([]);
  });

  it("switches selection to the next surviving project when the active one is deleted", async () => {
    const state = createMockClientState();
    state.projects = [P1, P2];
    const client = createMockClient(state);
    const projects = renderHookWithClient(() => useProjects(), client);
    const mutation = renderHookWithClient(() => useDeleteProject(), client, projects.queryClient);

    await waitFor(() => expect(projects.result.current.isSuccess).toBe(true));
    useWorkspaceSelectionStore.getState().selectProject("p1");

    mutation.result.current.mutate({ projectId: "p1" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(useWorkspaceSelectionStore.getState().selection.projectId).toBe("p2");
  });

  it("clears selection when the last project is deleted", async () => {
    const state = createMockClientState();
    state.projects = [P1];
    const client = createMockClient(state);
    const projects = renderHookWithClient(() => useProjects(), client);
    const mutation = renderHookWithClient(() => useDeleteProject(), client, projects.queryClient);

    await waitFor(() => expect(projects.result.current.isSuccess).toBe(true));
    useWorkspaceSelectionStore.getState().selectProject("p1");

    mutation.result.current.mutate({ projectId: "p1" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: null,
      taskId: null,
      sessionId: null,
    });
  });
});

describe("useCreateTask", () => {
  it("creates a task and selects it under its project", async () => {
    const state = createMockClientState();
    state.projects = [P1];
    const client = createMockClient(state);
    const projects = renderHookWithClient(() => useProjects(), client);
    const mutation = renderHookWithClient(() => useCreateTask(), client, projects.queryClient);

    await waitFor(() => expect(projects.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({ projectId: "p1", title: "New task", status: "todo" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.tasks).toHaveLength(1);
    const selection = useWorkspaceSelectionStore.getState().selection;
    expect(selection.projectId).toBe("p1");
    expect(selection.taskId).toBe(state.tasks[0]!.id);
  });

  it("expands the owning project so the new task is visible", async () => {
    const state = createMockClientState();
    state.projects = [P1];
    const client = createMockClient(state);
    const projects = renderHookWithClient(() => useProjects(), client);
    const mutation = renderHookWithClient(() => useCreateTask(), client, projects.queryClient);

    await waitFor(() => expect(projects.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({ projectId: "p1", title: "New task", status: "todo" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(useUiStore.getState().expandedProjects).toEqual(new Set(["p1"]));
  });
});

describe("useUpdateTask", () => {
  it("updates the task fields", async () => {
    const state = createMockClientState();
    state.tasks = [T1];
    const client = createMockClient(state);
    const tasks = renderHookWithClient(() => useTasks(), client);
    const mutation = renderHookWithClient(() => useUpdateTask(), client, tasks.queryClient);

    await waitFor(() => expect(tasks.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({ task: T1, title: "Renamed", status: "done" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.tasks[0]).toEqual({ id: "t1", projectId: "p1", title: "Renamed", status: "done" });
  });
});

describe("useDeleteTask", () => {
  it("cascades child sessions before deleting the task", async () => {
    const state = createMockClientState();
    state.tasks = [T1, T2];
    state.sessions = [S1, S2];
    const client = createMockClient(state);
    const tasks = renderHookWithClient(() => useTasks(), client);
    const sessions = renderHookWithClient(() => useSessions(), client, tasks.queryClient);
    const mutation = renderHookWithClient(() => useDeleteTask(), client, tasks.queryClient);

    await waitFor(() => expect(tasks.result.current.isSuccess).toBe(true));
    await waitFor(() => expect(sessions.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({ taskId: "t1" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.tasks.map((t) => t.id)).toEqual(["t2"]);
    expect(state.sessions).toEqual([]);
  });

  it("clears the task leg of the selection when the active task is deleted", async () => {
    const state = createMockClientState();
    state.tasks = [T1];
    const client = createMockClient(state);
    const tasks = renderHookWithClient(() => useTasks(), client);
    const mutation = renderHookWithClient(() => useDeleteTask(), client, tasks.queryClient);

    await waitFor(() => expect(tasks.result.current.isSuccess).toBe(true));
    useWorkspaceSelectionStore.getState().selectTask("t1", "p1");

    mutation.result.current.mutate({ taskId: "t1" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: null,
      sessionId: null,
    });
  });
});

describe("useCreateSession", () => {
  it("creates a session and selects it with its owning task and project", async () => {
    const state = createMockClientState();
    state.projects = [P1];
    state.tasks = [T1];
    const client = createMockClient(state);
    const newSessionRequests: Parameters<AcpClient["newSession"]>[0][] = [];
    const chatStore = createChatStore({
      newSession: async (request) => {
        newSessionRequests.push(request);
        return { sessionId: "agent-session-created" };
      },
      prompt: async () => ({ stopReason: "end_turn" }),
      cancel: async () => undefined,
      subscribe: () => () => undefined,
    });
    const projects = renderHookWithClient(() => useProjects(), client);
    const tasks = renderHookWithClient(() => useTasks(), client, projects.queryClient);
    const mutation = renderHookWithClient(
      () => useCreateSession(),
      client,
      projects.queryClient,
      chatStore,
    );

    await waitFor(() => expect(tasks.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({ taskId: "t1", agentId: "codex", status: "running" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.sessions).toHaveLength(1);
    expect(state.sessions[0]!.agentSessionId).toBe("agent-session-created");
    expect(newSessionRequests).toEqual([{ cwd: "/ora", mcpServers: [] }]);
    const selection = useWorkspaceSelectionStore.getState().selection;
    expect(selection).toEqual({
      projectId: "p1",
      taskId: "t1",
      sessionId: state.sessions[0]!.id,
    });
    // The session sits two levels down, so both ancestors have to open.
    expect(useUiStore.getState().expandedProjects).toEqual(new Set(["p1"]));
    expect(useUiStore.getState().expandedTasks).toEqual(new Set(["t1"]));
  });
});

describe("useUpdateSession", () => {
  it("updates the session fields", async () => {
    const state = createMockClientState();
    state.sessions = [S1];
    const client = createMockClient(state);
    const sessions = renderHookWithClient(() => useSessions(), client);
    const mutation = renderHookWithClient(() => useUpdateSession(), client, sessions.queryClient);

    await waitFor(() => expect(sessions.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({ session: S1, agentId: "claude", status: "stopped" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.sessions[0]).toEqual({
      id: "s1",
      taskId: "t1",
      agentId: "claude",
      agentSessionId: null,
      status: "stopped",
    });
  });
});

describe("useDeleteSession", () => {
  it("deletes the session and clears the session leg of the selection", async () => {
    const state = createMockClientState();
    state.sessions = [S1];
    const client = createMockClient(state);
    const sessions = renderHookWithClient(() => useSessions(), client);
    const mutation = renderHookWithClient(() => useDeleteSession(), client, sessions.queryClient);

    await waitFor(() => expect(sessions.result.current.isSuccess).toBe(true));
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");

    mutation.result.current.mutate({ sessionId: "s1" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.sessions).toEqual([]);
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: "t1",
      sessionId: null,
    });
  });
});
