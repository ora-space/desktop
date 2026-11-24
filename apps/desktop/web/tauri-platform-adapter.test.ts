import { cloudConnectionWire } from "./cloud-connection-fixtures.generated";
import { marketplaceAutoSyncWire } from "./marketplace-sync-fixtures.generated";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PathSelectionInProgressError } from "@ora/app-shell/platform";
import { createTauriPlatformAdapter } from "./tauri-platform-adapter";

const { onDragDropEventMock } = vi.hoisted(() => ({
  onDragDropEventMock: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: onDragDropEventMock }),
}));

const openMock = vi.mocked(open);
const saveMock = vi.mocked(save);
const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);

describe("TauriPlatformAdapter", () => {
  afterEach(() => vi.useRealTimers());

  beforeEach(() => {
    openMock.mockReset();
    saveMock.mockReset();
    invokeMock.mockReset();
    listenMock.mockReset();
    onDragDropEventMock.mockReset();
  });

  it("asks the host whether this installation is new", async () => {
    invokeMock.mockResolvedValueOnce(true);
    const { installation } = createTauriPlatformAdapter();

    await expect(installation?.isNew()).resolves.toBe(true);
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith("is_new_installation");
  });

  it("maps surface commands to the desktop command names and request shapes", async () => {
    const record = {
      instance: 7,
      pluginId: "ora.skill-hub",
      kind: "webview",
      title: "Example Hub",
      target: "embedded",
      state: "opening",
    };
    invokeMock.mockResolvedValueOnce({ embedded: true });
    invokeMock.mockResolvedValueOnce([record]);
    invokeMock.mockResolvedValueOnce(record);
    const adapter = createTauriPlatformAdapter();
    const surfaces = adapter.surfaces;

    await expect(surfaces.capabilities()).resolves.toEqual({ embedded: true });
    await expect(surfaces.list()).resolves.toEqual([record]);
    await expect(
      surfaces.open({ pluginId: "ora.skill-hub" }, "embedded"),
    ).resolves.toEqual(record);
    await surfaces.setBounds(7, { x: 1, y: 2, width: 3, height: 4, scale: 2 });
    await surfaces.setVisible(7, false);
    await surfaces.popout(7);
    await surfaces.dock(7);
    await surfaces.reload(7);
    await surfaces.close(7);

    expect(invokeMock.mock.calls).toEqual([
      ["surface_capabilities"],
      ["surface_list"],
      [
        "surface_open",
        {
          request: {
            pluginId: "ora.skill-hub",
            target: "embedded",
          },
        },
      ],
      [
        "surface_set_bounds",
        { request: { instance: 7, x: 1, y: 2, width: 3, height: 4, scale: 2 } },
      ],
      ["surface_set_visible", { request: { instance: 7, visible: false } }],
      ["surface_popout", { request: { instance: 7 } }],
      ["surface_dock", { request: { instance: 7 } }],
      ["surface_reload", { request: { instance: 7 } }],
      ["surface_close", { request: { instance: 7 } }],
    ]);
  });

  it("forwards surface lifecycle events and stops listening on unsubscribe", async () => {
    const stop = vi.fn();
    const listener = vi.fn();
    let forward: ((event: { payload: unknown }) => void) | undefined;
    listenMock.mockImplementation(async (_event, handler) => {
      forward = handler as (event: { payload: unknown }) => void;
      return stop;
    });
    const adapter = createTauriPlatformAdapter();

    const unsubscribe = await adapter.surfaces.onEvent(listener);
    forward?.({
      payload: {
        type: "downloadCompleted",
        instance: 7,
        pluginId: "ora.skill-hub",
        fileName: "skill.zip",
        path: "/downloads/skill.zip",
      },
    });
    unsubscribe();

    expect(listenMock).toHaveBeenCalledWith(
      "surface://event",
      expect.any(Function),
    );
    expect(listener).toHaveBeenCalledWith({
      type: "downloadCompleted",
      instance: 7,
      pluginId: "ora.skill-hub",
      fileName: "skill.zip",
      path: "/downloads/skill.zip",
    });
    expect(stop).toHaveBeenCalledOnce();
  });

  it("forwards marketplace install progress and stops listening on unsubscribe", async () => {
    const stop = vi.fn();
    const listener = vi.fn();
    let forward: ((event: { payload: unknown }) => void) | undefined;
    listenMock.mockImplementation(async (_event, handler) => {
      forward = handler as (event: { payload: unknown }) => void;
      return stop;
    });
    const adapter = createTauriPlatformAdapter();

    const unsubscribe =
      await adapter.pluginMarketplace.onInstallProgress(listener);
    forward?.({
      payload: {
        pluginId: "official/weather",
        downloaded: 4,
        total: 10,
      },
    });
    unsubscribe();

    expect(listenMock).toHaveBeenCalledWith(
      "plugin-install-progress",
      expect.any(Function),
    );
    expect(listener).toHaveBeenCalledWith({
      pluginId: "official/weather",
      downloaded: 4,
      total: 10,
    });
    expect(stop).toHaveBeenCalledOnce();
  });

  it("consumes Rust-serialized auto-sync events and releases the native listener", async () => {
    const stop = vi.fn();
    const listener = vi.fn();
    let forward: ((event: { payload: unknown }) => void) | undefined;
    listenMock.mockImplementation(async (_event, handler) => {
      forward = handler as (event: { payload: unknown }) => void;
      return stop;
    });
    const adapter = createTauriPlatformAdapter();
    const unsubscribe =
      await adapter.pluginMarketplace.onAutoSyncChanged(listener);
    for (const payload of marketplaceAutoSyncWire.payloads)
      forward?.({ payload });
    unsubscribe();

    expect(listenMock).toHaveBeenCalledWith(
      marketplaceAutoSyncWire.route,
      expect.any(Function),
    );
    expect(listener.mock.calls).toEqual([
      [{ kind: "started" }],
      [{ kind: "finished" }],
    ]);
    expect(stop).toHaveBeenCalledOnce();
  });

  it("consumes Rust-serialized Cloud connection changes and releases the native listener", async () => {
    const stop = vi.fn();
    const listener = vi.fn();
    let forward: ((event: { payload: unknown }) => void) | undefined;
    listenMock.mockImplementation(async (_event, handler) => {
      forward = handler as (event: { payload: unknown }) => void;
      return stop;
    });
    const adapter = createTauriPlatformAdapter();
    const unsubscribe = await adapter.cloudConnection.onChanged(listener);
    for (const payload of cloudConnectionWire.payloads) forward?.({ payload });
    unsubscribe();

    expect(listenMock).toHaveBeenCalledWith(
      cloudConnectionWire.route,
      expect.any(Function),
    );
    expect(listener.mock.calls).toEqual(
      cloudConnectionWire.payloads.map((payload) => [payload]),
    );
    expect(stop).toHaveBeenCalledOnce();
  });

  it("reads and updates the desktop worktree root through Tauri commands", async () => {
    invokeMock.mockResolvedValueOnce({ worktreeRoot: "/home/ora/worktrees" });
    const adapter = createTauriPlatformAdapter();

    await expect(adapter.worktreeStorage.getRoot()).resolves.toBe(
      "/home/ora/worktrees",
    );
    await adapter.worktreeStorage.setRoot("/mnt/worktrees");

    expect(invokeMock).toHaveBeenNthCalledWith(1, "get_worktree_root", {
      request: {},
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "set_worktree_root", {
      request: { worktreeRoot: "/mnt/worktrees" },
    });
  });

  it("maps directory selection and its initial path to the native dialog", async () => {
    openMock.mockResolvedValue("/home/ora/project");
    const adapter = createTauriPlatformAdapter();

    await expect(
      adapter.selectPath({ kind: "directory", initialPath: "/home/ora" }),
    ).resolves.toBe("/home/ora/project");
    expect(openMock).toHaveBeenCalledWith({
      directory: true,
      multiple: false,
      defaultPath: "/home/ora",
    });
  });

  it("saves exported workflow JSON at the native dialog destination", async () => {
    saveMock.mockResolvedValue("/home/ora/export.reactflow.json");
    const adapter = createTauriPlatformAdapter();

    await expect(
      adapter.saveTextFile({
        defaultFileName: "workflow.reactflow.json",
        content: '{"id":"workflow"}\n',
      }),
    ).resolves.toBe(true);

    expect(saveMock).toHaveBeenCalledWith({
      defaultPath: "workflow.reactflow.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    expect(invokeMock).toHaveBeenCalledWith("write_workflow_export", {
      request: {
        path: "/home/ora/export.reactflow.json",
        content: '{"id":"workflow"}\n',
      },
    });
  });

  it("forwards native OS file drops and reads the dropped path", async () => {
    const stop = vi.fn();
    onDragDropEventMock.mockResolvedValue(stop);
    invokeMock.mockResolvedValue({
      name: "import.reactflow.json",
      size: 12,
      content: '{"ok":true}',
    });
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const adapter = createTauriPlatformAdapter();
    const listener = vi.fn();
    const unsubscribe = adapter.subscribeOsFileDrop(listener);

    expect(onDragDropEventMock).toHaveBeenCalledOnce();
    const handler = onDragDropEventMock.mock.calls[0]?.[0] as (event: {
      payload: { type: string; paths?: string[] };
    }) => void;
    handler({ payload: { type: "over" } });
    handler({
      payload: { type: "drop", paths: ["/tmp/import.reactflow.json"] },
    });
    expect(listener).toHaveBeenCalledWith(["/tmp/import.reactflow.json"]);

    await expect(
      adapter.readOsTextFile("/tmp/import.reactflow.json"),
    ).resolves.toEqual({
      name: "import.reactflow.json",
      size: 12,
      content: '{"ok":true}',
    });
    expect(invokeMock).toHaveBeenCalledWith("read_workflow_import", {
      request: { path: "/tmp/import.reactflow.json" },
    });

    unsubscribe();
    await Promise.resolve();
    expect(stop).toHaveBeenCalledOnce();
    vi.unstubAllGlobals();
  });

  it("downloads today's diagnostic log to the native save destination", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 8, 3, 12));
    saveMock.mockResolvedValue("/home/ora/ora-logs-2026-09-03.log");
    const adapter = createTauriPlatformAdapter();

    await expect(adapter.diagnosticLogs.downloadToday()).resolves.toBe(true);

    expect(saveMock).toHaveBeenCalledWith({
      defaultPath: "ora-logs-2026-09-03.log",
      filters: [{ name: "Log", extensions: ["log"] }],
    });
    expect(invokeMock).toHaveBeenCalledWith("download_today_log", {
      request: { destination: "/home/ora/ora-logs-2026-09-03.log" },
    });
  });

  it("downloads one plugin's log under a sanitized suggested name", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 8, 3, 12));
    saveMock.mockResolvedValue("/home/ora/ora-plugin-claude-2026-09-03.log");
    const adapter = createTauriPlatformAdapter();

    await expect(
      adapter.diagnosticLogs.downloadPluginLog(
        "official/claude",
        "Claude Code",
      ),
    ).resolves.toBe(true);

    expect(saveMock).toHaveBeenCalledWith({
      defaultPath: "ora-plugin-Claude-Code-2026-09-03.log",
      filters: [{ name: "Log", extensions: ["log"] }],
    });
    expect(invokeMock).toHaveBeenCalledWith("download_plugin_log", {
      request: {
        pluginId: "official/claude",
        destination: "/home/ora/ora-plugin-claude-2026-09-03.log",
      },
    });
  });

  it("does not invoke the host when diagnostic log saving is cancelled", async () => {
    saveMock.mockResolvedValue(null);
    const adapter = createTauriPlatformAdapter();

    await expect(adapter.diagnosticLogs.downloadToday()).resolves.toBe(false);

    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("opens http URLs through the native host command", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    const adapter = createTauriPlatformAdapter();

    await adapter.openExternalUrl("https://example.com");

    expect(invokeMock).toHaveBeenCalledWith("open_external_url", {
      request: { url: "https://example.com" },
    });
  });

  it("returns null on cancellation and rejects concurrent native dialogs", async () => {
    const resolvers: Array<(path: string | null) => void> = [];
    const pendingOpen = new Promise<string | null>((resolve) => {
      resolvers.push(resolve);
    });
    openMock.mockReturnValue(pendingOpen);
    const adapter = createTauriPlatformAdapter();
    const firstSelection = adapter.selectPath({ kind: "file" });

    await expect(adapter.selectPath({ kind: "file" })).rejects.toBeInstanceOf(
      PathSelectionInProgressError,
    );
    resolvers[0]!(null);
    await expect(firstSelection).resolves.toBeNull();
  });
});
