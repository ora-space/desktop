import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { hasPlatformHostRenderer } from "../platform-host-renderer";
import { PathSelectionInProgressError } from "../types";
import { createTauriPlatformAdapter } from "./tauri-platform-adapter";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const openMock = vi.mocked(open);
const invokeMock = vi.mocked(invoke);

describe("TauriPlatformAdapter", () => {
  beforeEach(() => {
    openMock.mockReset();
    invokeMock.mockReset();
  });

  it("reads and updates the desktop worktree root through Tauri commands", async () => {
    invokeMock.mockResolvedValueOnce({ worktreeRoot: "/home/ora/worktrees" });
    const adapter = createTauriPlatformAdapter();

    await expect(adapter.worktreeStorage.getRoot()).resolves.toBe("/home/ora/worktrees");
    await adapter.worktreeStorage.setRoot("/mnt/worktrees");

    expect(invokeMock).toHaveBeenNthCalledWith(1, "get_desktop_config", { request: {} });
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

  it("does not expose the Web-only React path picker host", () => {
    const adapter = createTauriPlatformAdapter();

    expect(hasPlatformHostRenderer(adapter)).toBe(false);
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
