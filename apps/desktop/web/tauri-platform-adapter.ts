import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  PathSelectionInProgressError,
  type LocationActionsCapability,
  type LocationTarget,
  type PlatformAdapter,
  type SelectPathOptions,
  type SaveTextFileOptions,
  type SkillMarketplaceCapability,
  type SkillMarketplaceProvider,
  type SkillMarketplaceStatus,
  type WindowControlsCapability,
  type WindowManagerOs,
} from "@ora/app-shell/platform";

const SKILL_MARKETPLACE_STATUS_EVENT = "skill-marketplace://status";

/** Reads the host OS from the webview user agent without an async Tauri call. */
function detectWindowManagerOs(): WindowManagerOs | null {
  if (typeof navigator === "undefined") return null;
  const ua = navigator.userAgent;
  if (/Mac|iPhone|iPad|iPod/i.test(ua)) return "macos";
  if (/Windows|Win32|Win64|WOW64/i.test(ua)) return "windows";
  if (/Linux|X11|CrOS/i.test(ua)) return "linux";
  return null;
}

/** Creates the window-control capability used by the frameless Desktop shell. */
function createTauriWindowControls(): WindowControlsCapability {
  // The adapter is also instantiated by jsdom tests, where the Tauri IPC bridge
  // is absent and no native window controls can be exposed.
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    return { kind: "none" };
  }

  const os = detectWindowManagerOs();
  if (os === null || os === "macos") {
    return { kind: "none" };
  }

  const appWindow = getCurrentWindow();
  return {
    kind: "overlay",
    os,
    minimize: () => appWindow.minimize(),
    toggleMaximize: () => appWindow.toggleMaximize(),
    close: () => appWindow.close(),
    isMaximized: () => appWindow.isMaximized(),
    subscribeMaximized: (listener) => {
      let active = true;
      // Tauri has no dedicated maximize event, so every resize re-reads the flag.
      const unlisten = appWindow.onResized(() => {
        void appWindow.isMaximized().then((maximized) => {
          if (active) listener(maximized);
        });
      });
      return () => {
        active = false;
        void unlisten.then((stop) => stop());
      };
    },
  };
}

/** Wires the location handoff commands exposed by the Desktop runtime. */
function createTauriLocationActions(): LocationActionsCapability {
  return {
    resolveTaskCwd: (taskId) =>
      invoke<{ path: string }>("resolve_task_cwd", {
        request: { taskId },
      }).then((response) => response.path),
    open: (target: LocationTarget, path: string) =>
      invoke("open_location", { request: { target, path } }),
  };
}

/** Implements the app-shell host capabilities with Tauri APIs and commands. */
export class TauriPlatformAdapter implements PlatformAdapter {
  private selectionInProgress = false;

  readonly windowControls: WindowControlsCapability =
    createTauriWindowControls();

  readonly locationActions: LocationActionsCapability =
    createTauriLocationActions();

  readonly skillMarketplace: SkillMarketplaceCapability = {
    open: (provider: SkillMarketplaceProvider) =>
      invoke("open_skill_marketplace", { request: { provider } }),
    onStatus: (listener) =>
      listen<SkillMarketplaceStatus>(
        SKILL_MARKETPLACE_STATUS_EVENT,
        (event) => {
          listener(event.payload);
        },
      ),
  };

  readonly worktreeStorage = {
    getRoot: async (): Promise<string> => {
      const config = await invoke<{ worktreeRoot: string }>(
        "get_desktop_config",
        {
          request: {},
        },
      );
      return config.worktreeRoot;
    },
    setRoot: async (path: string): Promise<void> => {
      await invoke("set_worktree_root", {
        request: { worktreeRoot: path },
      });
    },
  };

  /** Opens one native single-selection dialog configured for a file or directory. */
  async selectPath(options: SelectPathOptions): Promise<string | null> {
    if (this.selectionInProgress) {
      throw new PathSelectionInProgressError();
    }

    this.selectionInProgress = true;
    try {
      return await open({
        directory: options.kind === "directory",
        multiple: false,
        defaultPath: options.initialPath,
      });
    } finally {
      this.selectionInProgress = false;
    }
  }

  /** Opens the native save dialog, then writes the user-selected workflow export. */
  async saveTextFile(options: SaveTextFileOptions): Promise<boolean> {
    if (this.selectionInProgress) {
      throw new PathSelectionInProgressError();
    }

    this.selectionInProgress = true;
    try {
      const path = await save({
        defaultPath: options.defaultFileName,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (path === null) {
        return false;
      }
      await invoke("write_workflow_export", {
        request: { path, content: options.content },
      });
      return true;
    } finally {
      this.selectionInProgress = false;
    }
  }
}

/** Creates the Desktop host adapter without runtime platform auto-detection. */
export function createTauriPlatformAdapter(): TauriPlatformAdapter {
  return new TauriPlatformAdapter();
}
