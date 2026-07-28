import { describe, expect, it, vi } from "vitest";
import { ContractTransportError } from "@ora/contracts";
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

  it("maps model discovery to the dedicated desktop command", async () => {
    const invoke = vi.fn().mockResolvedValue({ groups: [] });
    const transport = createTauriTransport(invoke);

    await expect(transport.send({
      operationName: "listAgentModels",
      request: {},
      method: "GET",
      path: "/api/agent-models",
      body: undefined,
      headers: {},
    })).resolves.toEqual({ groups: [] });
    expect(invoke).toHaveBeenCalledWith("list_agent_models", { request: {} });
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
    ).rejects.toMatchObject({ code: "unsupported_operation", status: null });
    expect(invoke).not.toHaveBeenCalled();
  });

  it("normalizes structured command errors", async () => {
    const invoke = vi.fn().mockRejectedValue({
      code: "project_not_found",
      message: "project not found: project-1",
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
      expect(error).toBeInstanceOf(ContractTransportError);
      expect(error).toMatchObject({
        code: "project_not_found",
        status: null,
        responseBody: {
          code: "project_not_found",
          message: "project not found: project-1",
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
      expect.objectContaining({ code: "stream_already_consumed" }),
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
      request: { sessionId: "session-1", text: "hello" },
      method: "POST",
      path: "/api/sessions/session-1/prompt",
      body: { text: "hello" },
      headers: { "content-type": "application/json" },
    });

    await expect(async () => {
      for await (const event of stream) {
        // The transport detects overflow before yielding buffered business events.
        void event;
      }
    }).rejects.toMatchObject({ code: "stream_queue_overflow" });
  });
});
