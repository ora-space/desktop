import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PathSelectionInProgressError } from "@ora/app-shell/platform";
import { createTauriPlatformAdapter } from "./tauri-platform-adapter";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const openMock = vi.mocked(open);
const saveMock = vi.mocked(save);
const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);

describe("TauriPlatformAdapter", () => {
  beforeEach(() => {
    openMock.mockReset();
    saveMock.mockReset();
    invokeMock.mockReset();
    listenMock.mockReset();
  });

  it("opens provider-specific marketplaces and forwards native download status events", async () => {
    const stop = vi.fn();
    const listener = vi.fn();
    let forwardStatus: ((event: { payload: unknown }) => void) | undefined;
    listenMock.mockImplementation(async (_event, handler) => {
      forwardStatus = handler as (event: { payload: unknown }) => void;
      return stop;
    });
    const adapter = createTauriPlatformAdapter();
    const marketplace = adapter.skillMarketplace;

    await marketplace.open("huaweiAgentCenter");
    const unsubscribe = await marketplace.onStatus(listener);
    forwardStatus?.({
      payload: {
        status: "downloaded",
        provider: "huaweiAgentCenter",
        fileName: "skill.zip",
        archivePath: "/app-data/skill-downloads/skill.zip",
      },
    });
    unsubscribe();

    expect(invokeMock).toHaveBeenCalledWith("open_skill_marketplace", {
      request: { provider: "huaweiAgentCenter" },
    });
    expect(listenMock).toHaveBeenCalledWith(
      "skill-marketplace://status",
      expect.any(Function),
    );
    expect(listener).toHaveBeenCalledWith({
      status: "downloaded",
      provider: "huaweiAgentCenter",
      fileName: "skill.zip",
      archivePath: "/app-data/skill-downloads/skill.zip",
    });
    expect(stop).toHaveBeenCalledOnce();
  });

  it("reads and updates the desktop worktree root through Tauri commands", async () => {
    invokeMock.mockResolvedValueOnce({ worktreeRoot: "/home/ora/worktrees" });
    const adapter = createTauriPlatformAdapter();

    await expect(adapter.worktreeStorage.getRoot()).resolves.toBe(
      "/home/ora/worktrees",
    );
    await adapter.worktreeStorage.setRoot("/mnt/worktrees");

    expect(invokeMock).toHaveBeenNthCalledWith(1, "get_desktop_config", {
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
