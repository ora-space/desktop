import { describe, it, expect } from "vitest";
import { waitFor } from "@testing-library/react";
import { useProjects } from "./use-projects";
import { useTasks } from "./use-tasks";
import { useSessions } from "./use-sessions";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { renderHookWithClient } from "../../test/hook-harness";

describe("useProjects", () => {
  it("returns the project list from the client", async () => {
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora", rootPath: "/ora" }];
    const client = createMockClient(state);
    const { result } = renderHookWithClient(() => useProjects(), client);
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual([
      { id: "p1", name: "Ora", rootPath: "/ora" },
    ]);
  });

  it("starts pending with no data", () => {
    const state = createMockClientState();
    const client = createMockClient(state);
    const { result } = renderHookWithClient(() => useProjects(), client);
    expect(result.current.isPending).toBe(true);
    expect(result.current.data).toBeUndefined();
  });

  it("surfaces transport errors as isError", async () => {
    const state = createMockClientState();
    const client = createMockClient(state);
    (client.project as unknown as { list: () => Promise<never> }).list =
      async () => {
        throw new Error("boom");
      };
    const { result } = renderHookWithClient(() => useProjects(), client);
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.error).toBeInstanceOf(Error);
  });
});

describe("useTasks", () => {
  it("returns the task list from the client", async () => {
    const state = createMockClientState();
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        title: "Refactor",
        workspaceMode: "worktree",
        type: "default",
        workflowRunId: null,
      },
    ];
    const client = createMockClient(state);
    const { result } = renderHookWithClient(() => useTasks(), client);
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual([
      {
        id: "t1",
        projectId: "p1",
        title: "Refactor",
        workspaceMode: "worktree",
        type: "default",
        workflowRunId: null,
      },
    ]);
  });
});

describe("useSessions", () => {
  it("returns the session list from the client", async () => {
    const state = createMockClientState();
    state.sessions = [
      {
        id: "s1",
        taskId: "t1",
        agentCli: "open_code",
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const client = createMockClient(state);
    const { result } = renderHookWithClient(() => useSessions(), client);
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual([
      {
        id: "s1",
        taskId: "t1",
        agentCli: "open_code",
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ]);
  });
});
