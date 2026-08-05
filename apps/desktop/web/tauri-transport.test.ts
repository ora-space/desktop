import { describe, expect, it, vi } from "vitest";
import { LocalTransportError, RemoteContractError } from "@ora/contracts";
import { createTauriTransport } from "./tauri-transport";

describe("createTauriTransport", () => {
  it("maps supported operations and forwards the complete request", async () => {
    const invoke = vi.fn().mockResolvedValue({ projects: [] });
    const transport = createTauriTransport(invoke, () => ({ onmessage: () => undefined }));
    const request = {
      operationName: "listProjects",
      request: {},
      method: "GET" as const,
      path: "/api/projects",
      body: undefined,
      headers: {},
    };

    await expect(transport.send(request)).resolves.toEqual({ projects: [] });
    expect(invoke).toHaveBeenCalledWith("list_projects", { request: {} });
  });

  it("maps workspace directory reads to the dedicated desktop command", async () => {
    const response = { path: "src", entries: [] };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(transport.send({
      operationName: "listWorkspaceDirectory",
      request: { taskId: "task-1", path: "src" },
      method: "POST",
      path: "/api/tasks/task-1/files/list",
      body: { taskId: "task-1", path: "src" },
      headers: { "content-type": "application/json" },
    })).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("list_workspace_directory", {
      request: { taskId: "task-1", path: "src" },
    });
  });

  it("maps task diff reads to the shared desktop backend command", async () => {
    const response = {
      baseCommitId: "base",
      headCommitId: "head",
      diffId: "diff-1",
      patch: "diff --git a/README.md b/README.md",
    };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);
    const request = { taskId: "task-1", scope: "branch" as const };

    await expect(transport.send({
      operationName: "getTaskDiff",
      request,
      method: "GET",
      path: "/api/tasks/task-1/diff?scope=branch",
      body: undefined,
      headers: {},
    })).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("get_task_diff", { request });
  });

  it.each([
    ["switchSessionAgent", "switch_session_agent", { sessionId: "s1", agentCli: "claude" }],
    ["resumeSessionHistory", "resume_session_history", { sessionId: "s1" }],
  ] as const)("routes %s to its desktop command", async (operationName, command, request) => {
    const invoke = vi.fn().mockResolvedValue({});
    const transport = createTauriTransport(invoke, () => ({ onmessage: () => undefined }));

    await transport.send({
      operationName,
      request,
      method: "POST",
      path: `/api/sessions/${request.sessionId}`,
      body: request,
      headers: {},
    });

    expect(invoke).toHaveBeenCalledWith(command, { request });
  });

  it("maps task workspaces and spec operations to their Desktop commands", async () => {
    const invoke = vi.fn()
      .mockResolvedValueOnce({ rootPath: "C:/repo/.ora-worktrees/task-1", branchName: "ora/task-1" })
      .mockResolvedValueOnce({ sources: [], documents: [], truncated: false });
    const transport = createTauriTransport(invoke);

    await expect(transport.send({
      operationName: "getTaskWorkspace",
      request: { taskId: "task-1" },
      method: "GET",
      path: "/api/tasks/task-1/workspace",
      body: undefined,
      headers: {},
    })).resolves.toMatchObject({ branchName: "ora/task-1" });
    await expect(transport.send({
      operationName: "getSpecCatalog",
      request: { target: { kind: "task", taskId: "task-1" } },
      method: "POST",
      path: "/api/specs/catalog",
      body: { target: { kind: "task", taskId: "task-1" } },
      headers: { "content-type": "application/json" },
    })).resolves.toEqual({ sources: [], documents: [], truncated: false });
    expect(invoke).toHaveBeenNthCalledWith(1, "get_task_workspace", {
      request: { taskId: "task-1" },
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "get_spec_catalog", {
      request: { target: { kind: "task", taskId: "task-1" } },
    });
  });

  it("rejects explicitly unsupported operations before invoking Rust", async () => {
    const invoke = vi.fn();
    const transport = createTauriTransport(invoke, () => ({ onmessage: () => undefined }));

    await expect(
      transport.send({
        operationName: "listDirectory",
        request: { path: "/tmp" },
        method: "GET",
        path: "/api/file-system/directory?path=%2Ftmp",
        body: undefined,
        headers: {},
      }),
    ).rejects.toMatchObject({ kind: "unsupported_operation" });
    expect(invoke).not.toHaveBeenCalled();
  });

  it("streams spec watcher events through the shared channel lifecycle", async () => {
    const invoke = vi.fn().mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "stream_contract") {
        expect(args.operationName).toBe("watchSpecs");
        const channel = args.onEvent as { onmessage: (frame: unknown) => void };
        queueMicrotask(() => {
          channel.onmessage({ type: "data", data: { changes: [{ kind: "rescanRequired", path: "" }] } });
          channel.onmessage({ type: "end" });
        });
      }
    });
    const stream = createTauriTransport(
      invoke,
      () => ({ onmessage: () => undefined }),
    ).stream<{ changes: Array<{ kind: string; path: string }> }>({
      operationName: "watchSpecs",
      request: { target: { kind: "project", projectId: "project-1" } },
      method: "POST",
      path: "/api/specs/watch",
      body: { target: { kind: "project", projectId: "project-1" } },
      headers: { "content-type": "application/json" },
    });

    const events = [];
    for await (const event of stream) events.push(event);

    expect(events).toEqual([{ changes: [{ kind: "rescanRequired", path: "" }] }]);
    expect(invoke).toHaveBeenCalledWith("cancel_contract_stream", expect.any(Object));
  });

  it("normalizes structured command errors", async () => {
    const invoke = vi.fn().mockRejectedValue({
      code: "project_not_found",
      params: {},
      requestId: "550e8400-e29b-41d4-a716-446655440000",
    });
    const transport = createTauriTransport(invoke);

    try {
      await transport.send({
        operationName: "getProject",
        request: { projectId: "project-1" },
        method: "GET",
        path: "/api/projects/project-1",
        body: undefined,
        headers: {},
      });
      throw new Error("expected transport to reject");
    } catch (error) {
      expect(error).toBeInstanceOf(RemoteContractError);
      expect(error).toMatchObject({
        code: "project_not_found",
        status: null,
        responseBody: {
          code: "project_not_found",
          params: {},
          requestId: "550e8400-e29b-41d4-a716-446655440000",
        },
      });
    }
  });

  it("starts channel streams lazily and forwards ordered data until end", async () => {
    const invoke = vi.fn().mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "stream_contract") {
        const channel = args.onEvent as { onmessage: (frame: unknown) => void };
        queueMicrotask(() => {
          channel.onmessage({ type: "data", data: { value: 1 } });
          channel.onmessage({ type: "end" });
        });
      }
    });
    const transport = createTauriTransport(
      invoke,
      () => ({ onmessage: () => undefined }),
    );
    const stream = transport.stream<{ value: number }>({
      operationName: "loadSession",
      request: { sessionId: "session-1" },
      method: "POST",
      path: "/api/sessions/session-1/load",
      body: undefined,
      headers: {},
    });

    expect(invoke).not.toHaveBeenCalled();
    const events = [];
    for await (const event of stream) events.push(event);

    expect(events).toEqual([{ value: 1 }]);
    expect(invoke).toHaveBeenCalledWith("stream_contract", expect.objectContaining({
      operationName: "loadSession",
      request: { sessionId: "session-1" },
    }));
    expect(invoke).toHaveBeenCalledWith("cancel_contract_stream", expect.any(Object));
    expect(() => stream[Symbol.asyncIterator]()).toThrowError(
      expect.objectContaining({ kind: "stream_already_consumed" }),
    );
  });

  it("fails a channel stream when its bounded consumer queue overflows", async () => {
    const invoke = vi.fn().mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "stream_contract") {
        const channel = args.onEvent as { onmessage: (frame: unknown) => void };
        for (let index = 0; index < 257; index += 1) {
          channel.onmessage({ type: "data", data: { index } });
        }
      }
    });
    const stream = createTauriTransport(
      invoke,
      () => ({ onmessage: () => undefined }),
    ).stream({
      operationName: "promptSession",
      request: { sessionId: "session-1", prompt: [{ type: "text", text: "hello" }] },
      method: "POST",
      path: "/api/sessions/session-1/prompt",
      body: { prompt: [{ type: "text", text: "hello" }] },
      headers: { "content-type": "application/json" },
    });

    await expect(async () => {
      for await (const event of stream) {
        // The transport detects overflow before yielding buffered business events.
        void event;
      }
    }).rejects.toBeInstanceOf(LocalTransportError);
  });
});
