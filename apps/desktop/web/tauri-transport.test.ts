import { describe, expect, it, vi } from "vitest";
import { LocalTransportError, RemoteContractError } from "@ora/contracts";
import { createTauriTransport } from "./tauri-transport";

describe("createTauriTransport", () => {
  it("maps supported operations and forwards the complete request", async () => {
    const invoke = vi.fn().mockResolvedValue({ projects: [] });
    const transport = createTauriTransport(invoke, () => ({
      onmessage: () => undefined,
    }));
    const request = {
      operationName: "listProjects",
      request: {},
    };

    await expect(transport.send(request)).resolves.toEqual({ projects: [] });
    expect(invoke).toHaveBeenCalledWith("list_projects", { request: {} });
  });

  it("maps workspace listing to the Desktop command", async () => {
    const response = { workspaces: [] };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "listWorkspaces",
        request: {},
      }),
    ).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("list_workspaces", { request: {} });
  });

  it("maps workflow run rename to the Desktop command", async () => {
    const response = { workflowRun: {} };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "renameWorkflowRun",
        request: {},
      }),
    ).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("rename_workflow_run", {
      request: {},
    });
  });

  it("maps installed plugin discovery to the Desktop snapshot command", async () => {
    const response = { plugins: [] };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "listInstalledPlugins",
        request: {},
      }),
    ).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("list_installed_plugins", {
      request: {},
    });
  });
  it("maps available plugin discovery to the Desktop registry command", async () => {
    const response = { updatedAt: 0n, plugins: [] };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "listAvailablePlugins",
        request: {},
      }),
    ).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("list_available_plugins", {
      request: {},
    });
  });
  it("maps marketplace sync to the Desktop registry command", async () => {
    const response = { updatedAt: 0n, plugins: [] };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "syncAvailablePlugins",
        request: {},
      }),
    ).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("sync_available_plugins", {
      request: {},
    });
  });
  it("reads a marketplace plugin README through the Desktop plugin command", async () => {
    const response = { readme: "# Weather" };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "readPluginReadme",
        request: { pluginId: "official/weather" },
      }),
    ).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("read_plugin_readme", {
      request: { pluginId: "official/weather" },
    });
  });
  it("lists marketplace sources through the Desktop plugin command", async () => {
    const response = { sources: [] };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "listMarketplaceSources",
        request: {},
      }),
    ).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("list_marketplace_sources", {
      request: {},
    });
  });
  it("maps marketplace install to the Desktop plugin command", async () => {
    const response = { pluginId: "official/weather" };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "installPlugin",
        request: { pluginId: "official/weather" },
      }),
    ).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("install_plugin", {
      request: { pluginId: "official/weather" },
    });
  });
  it("maps marketplace plugin updates to the Desktop plugin command", async () => {
    const response = { pluginId: "official/weather" };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "updatePlugin",
        request: { pluginId: "official/weather" },
      }),
    ).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("update_plugin", {
      request: { pluginId: "official/weather" },
    });
  });
  it("maps local archive import to the Desktop plugin command", async () => {
    const response = { pluginId: "official/weather" };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "importPlugin",
        request: { path: "C:/downloads/weather.orax" },
      }),
    ).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("import_plugin", {
      request: { path: "C:/downloads/weather.orax" },
    });
  });
  it("maps workspace directory reads to the dedicated desktop command", async () => {
    const response = { path: "src", entries: [] };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "listWorkspaceDirectory",
        request: { taskId: "task-1", path: "src" },
      }),
    ).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("list_workspace_directory", {
      request: { taskId: "task-1", path: "src" },
    });
  });

  it("maps workspace diff reads to the shared desktop backend command", async () => {
    const response = {
      baseCommitId: "base",
      headCommitId: "head",
      patch: "diff --git a/README.md b/README.md",
    };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);
    const request = { workspaceId: "workspace-1", scope: "branch" as const };

    await expect(
      transport.send({
        operationName: "getWorkspaceDiff",
        request,
      }),
    ).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("get_workspace_diff", { request });
  });

  it.each([
    [
      "switchSessionAgent",
      "switch_session_agent",
      { sessionId: "s1", agentRef: "ora-space.claude" },
    ],
    ["resumeSessionHistory", "resume_session_history", { sessionId: "s1" }],
    ["prepareAgentImport", "prepare_agent_import", { content: "# Role" }],
    [
      "commitAgentImport",
      "commit_agent_import",
      {
        content: "# Role",
        decision: null,
        expectedAgentId: null,
        expectedUpdatedAt: null,
      },
    ],
    ["listWorkflows", "list_workflows", {}],
    ["getDraft", "get_workflow_draft", { workflowId: "wf-1" }],
    [
      "createWorkflowRun",
      "create_workflow_run",
      { workflowId: "wf-1", projectId: "project-1", locale: "zh-CN" },
    ],
    ["listWorkflowNodeRuns", "list_workflow_node_runs", { runId: "run-1" }],
  ] as const)(
    "routes %s to its desktop command",
    async (operationName, command, request) => {
      const invoke = vi.fn().mockResolvedValue({});
      const transport = createTauriTransport(invoke, () => ({
        onmessage: () => undefined,
      }));

      await transport.send({
        operationName,
        request,
      });

      expect(invoke).toHaveBeenCalledWith(command, { request });
    },
  );

  it("maps task workspaces and spec operations to their Desktop commands", async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce({
        rootPath: "C:/repo/.ora-worktrees/task-1",
        branchName: "ora/task-1",
      })
      .mockResolvedValueOnce({ documents: [], truncated: false });
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "getTaskWorkspace",
        request: { taskId: "task-1" },
      }),
    ).resolves.toMatchObject({ branchName: "ora/task-1" });
    await expect(
      transport.send({
        operationName: "getSpecCatalog",
        request: { target: { kind: "task", taskId: "task-1" } },
      }),
    ).resolves.toEqual({ documents: [], truncated: false });
    expect(invoke).toHaveBeenNthCalledWith(1, "get_task_workspace", {
      request: { taskId: "task-1" },
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "get_spec_catalog", {
      request: { target: { kind: "task", taskId: "task-1" } },
    });
  });

  it("maps runtime log-level reads and updates to Desktop commands", async () => {
    const response = {
      configuredLevel: "debug",
      effectiveLevel: "debug",
      startupOverride: null,
    };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "getRuntimeLogLevel",
        request: {},
      }),
    ).resolves.toEqual(response);
    await expect(
      transport.send({
        operationName: "setRuntimeLogLevel",
        request: { level: "debug" },
      }),
    ).resolves.toEqual(response);

    expect(invoke).toHaveBeenNthCalledWith(1, "get_runtime_log_level", {
      request: {},
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "set_runtime_log_level", {
      request: { level: "debug" },
    });
  });

  it("maps developer-mode reads and updates to Desktop commands", async () => {
    const response = { enabled: true };
    const invoke = vi.fn().mockResolvedValue(response);
    const transport = createTauriTransport(invoke);

    await expect(
      transport.send({
        operationName: "getDeveloperMode",
        request: {},
      }),
    ).resolves.toEqual(response);
    await expect(
      transport.send({
        operationName: "setDeveloperMode",
        request: { enabled: true },
      }),
    ).resolves.toEqual(response);

    expect(invoke).toHaveBeenNthCalledWith(1, "get_developer_mode", {
      request: {},
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "set_developer_mode", {
      request: { enabled: true },
    });
  });

  it("streams spec watcher events through the shared channel lifecycle", async () => {
    const invoke = vi
      .fn()
      .mockImplementation(
        async (command: string, args: Record<string, unknown>) => {
          if (command === "stream_contract") {
            expect(args.operationName).toBe("watchSpecs");
            const channel = args.onEvent as {
              onmessage: (frame: unknown) => void;
            };
            queueMicrotask(() => {
              channel.onmessage({
                type: "data",
                data: { changes: [{ kind: "rescanRequired", path: "" }] },
              });
              channel.onmessage({ type: "end" });
            });
          }
        },
      );
    const stream = createTauriTransport(invoke, () => ({
      onmessage: () => undefined,
    })).stream<{ changes: Array<{ kind: string; path: string }> }>({
      operationName: "watchSpecs",
      request: { target: { kind: "project", projectId: "project-1" } },
    });

    const events = [];
    for await (const event of stream) events.push(event);

    expect(events).toEqual([
      { changes: [{ kind: "rescanRequired", path: "" }] },
    ]);
    expect(invoke).toHaveBeenCalledWith(
      "cancel_contract_stream",
      expect.any(Object),
    );
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
      });
      throw new Error("expected transport to reject");
    } catch (error) {
      expect(error).toBeInstanceOf(RemoteContractError);
      expect(error).toMatchObject({
        code: "project_not_found",
        rawPayload: {
          code: "project_not_found",
          params: {},
          requestId: "550e8400-e29b-41d4-a716-446655440000",
        },
      });
    }
  });

  it("starts channel streams lazily and forwards ordered data until end", async () => {
    const invoke = vi
      .fn()
      .mockImplementation(
        async (command: string, args: Record<string, unknown>) => {
          if (command === "stream_contract") {
            const channel = args.onEvent as {
              onmessage: (frame: unknown) => void;
            };
            queueMicrotask(() => {
              channel.onmessage({ type: "data", data: { value: 1 } });
              channel.onmessage({ type: "end" });
            });
          }
        },
      );
    const transport = createTauriTransport(invoke, () => ({
      onmessage: () => undefined,
    }));
    const stream = transport.stream<{ value: number }>({
      operationName: "loadSession",
      request: { sessionId: "session-1" },
    });

    expect(invoke).not.toHaveBeenCalled();
    const events = [];
    for await (const event of stream) events.push(event);

    expect(events).toEqual([{ value: 1 }]);
    expect(invoke).toHaveBeenCalledWith(
      "stream_contract",
      expect.objectContaining({
        operationName: "loadSession",
        request: { sessionId: "session-1" },
      }),
    );
    expect(invoke).toHaveBeenCalledWith(
      "cancel_contract_stream",
      expect.any(Object),
    );
    expect(() => stream[Symbol.asyncIterator]()).toThrowError(
      expect.objectContaining({ kind: "stream_already_consumed" }),
    );
  });

  it("fails a channel stream when its bounded consumer queue overflows", async () => {
    const invoke = vi
      .fn()
      .mockImplementation(
        async (command: string, args: Record<string, unknown>) => {
          if (command === "stream_contract") {
            const channel = args.onEvent as {
              onmessage: (frame: unknown) => void;
            };
            for (let index = 0; index < 257; index += 1) {
              channel.onmessage({ type: "data", data: { index } });
            }
          }
        },
      );
    const stream = createTauriTransport(invoke, () => ({
      onmessage: () => undefined,
    })).stream({
      operationName: "promptSession",
      request: {
        sessionId: "session-1",
        prompt: [{ type: "text", text: "hello" }],
      },
    });

    await expect(async () => {
      for await (const event of stream) {
        // The transport detects overflow before yielding buffered business events.
        void event;
      }
    }).rejects.toBeInstanceOf(LocalTransportError);
  });
});
