import { describe, it, expect } from "vitest";
import { waitFor } from "@testing-library/react";
import { useProjects } from "./use-projects";
import { useTasks } from "./use-tasks";
import { useSessions } from "./use-sessions";
import { createTestClient } from "../../test/contracts-transport";
import {
  createWorkspaceMemory,
  workspaceHandlers,
} from "../../test/memory/workspaces";
import {
  createSessionMemory,
  sessionHandlers,
} from "../../test/memory/sessions";
import { renderHookWithClient } from "../../test/hook-harness";
import { AGENT_REF } from "../../test/agent-identity";

describe("useProjects", () => {
  it("returns the project list from the client", async () => {
    const state = createWorkspaceMemory();
    state.projects = [{ id: "p1", name: "Ora" }];
    const client = createTestClient(workspaceHandlers(state));
    const { result } = renderHookWithClient(() => useProjects(), client);
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual([{ id: "p1", name: "Ora" }]);
  });

  it("starts pending with no data", () => {
    const state = createWorkspaceMemory();
    const client = createTestClient(workspaceHandlers(state));
    const { result } = renderHookWithClient(() => useProjects(), client);
    expect(result.current.isPending).toBe(true);
    expect(result.current.data).toBeUndefined();
  });

  it("surfaces transport errors as isError", async () => {
    const client = createTestClient({
      listProjects: async () => {
        throw new Error("boom");
      },
    });
    const { result } = renderHookWithClient(() => useProjects(), client);
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.error).toBeInstanceOf(Error);
  });
});

describe("useTasks", () => {
  it("returns the task list from the client", async () => {
    const state = createWorkspaceMemory();
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Refactor",
      },
    ];
    const client = createTestClient(workspaceHandlers(state));
    const { result } = renderHookWithClient(() => useTasks(), client);
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual([
      {
        id: "t1",
        projectId: "p1",
        title: "Refactor",
        workspaceId: "workspace-t1",
      },
    ]);
  });
});

describe("useSessions", () => {
  it("returns the session list from the client", async () => {
    const state = createSessionMemory();
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
    const client = createTestClient(sessionHandlers(state));
    const { result } = renderHookWithClient(() => useSessions(), client);
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual([
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ]);
  });
});
