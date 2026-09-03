import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  PathSelectionInProgressError,
  type LocationActionsCapability,
  type DesktopUpdateCapability,
  type DesktopUpdateStatus,
  type DiagnosticLogsCapability,
  type LocationTarget,
  type PlatformAdapter,
  type PluginInstallProgress,
  type PluginMarketplaceCapability,
  type SelectPathOptions,
  type SaveTextFileOptions,
  type SelectSavePathOptions,
  type DownloadAction,
  type ResolveDownloadOutcome,
  type SurfaceBounds,
  type SurfaceCapability,
  type SurfaceEvent,
  type SurfaceOpenTarget,
  type SurfaceRecord,
  type SurfaceTarget,
  type WindowControlsCapability,
  type WindowManagerOs,
} from "@ora/app-shell/platform";

const SURFACE_EVENT = "surface://event";
const DESKTOP_UPDATE_EVENT = "desktop-update-status-changed";
const PLUGIN_INSTALL_PROGRESS_EVENT = "plugin-install-progress";

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
    resolveWorkspaceCwd: (workspaceId) =>
      invoke<{ path: string }>("resolve_workspace_cwd", {
        request: { workspaceId },
      }).then((response) => response.path),
    open: (target: LocationTarget, path: string) =>
      invoke("open_location", { request: { target, path } }),
  };
}

/** Wires the native updater status and installation commands into the shared shell. */
function createTauriUpdates(): DesktopUpdateCapability {
  return {
    getStatus: () => invoke<DesktopUpdateStatus>("get_desktop_update_status"),
    install: () => invoke("install_desktop_update"),
    check: () => invoke("check_desktop_update"),
    onStatus: (listener) =>
      listen<DesktopUpdateStatus>(DESKTOP_UPDATE_EVENT, (event) => {
        listener(event.payload);
      }),
  };
}

/** Wires marketplace package transfer progress into the shared settings UI. */
function createTauriPluginMarketplace(): PluginMarketplaceCapability {
  return {
    onInstallProgress: (listener) =>
      listen<PluginInstallProgress>(PLUGIN_INSTALL_PROGRESS_EVENT, (event) => {
        listener(event.payload);
      }),
  };
}

/** Wires the plugin surface commands and lifecycle event stream exposed by the Desktop runtime. */
function createTauriSurfaces(): SurfaceCapability {
  return {
    capabilities: () => invoke<{ embedded: boolean }>("surface_capabilities"),
    list: () => invoke<SurfaceRecord[]>("surface_list"),
    open: (target: SurfaceOpenTarget, mount: SurfaceTarget) =>
      invoke<SurfaceRecord>("surface_open", {
        request: {
          pluginId: target.pluginId,
          target: mount,
        },
      }),
    close: (instance: number) =>
      invoke("surface_close", { request: { instance } }),
    resolveDownload: (
      downloadId: number,
      action: DownloadAction,
      destination?: string,
    ) =>
      invoke<ResolveDownloadOutcome>("surface_resolve_download", {
        request: { downloadId, action, destination },
      }),
    discardDownload: (downloadId: number) =>
      invoke("surface_discard_download", { request: { downloadId } }),
    setBounds: (instance: number, bounds: SurfaceBounds) =>
      invoke("surface_set_bounds", { request: { instance, ...bounds } }),
    setVisible: (instance: number, visible: boolean) =>
      invoke("surface_set_visible", { request: { instance, visible } }),
    popout: (instance: number) =>
      invoke("surface_popout", { request: { instance } }),
    dock: (instance: number) =>
      invoke("surface_dock", { request: { instance } }),
    reload: (instance: number) =>
      invoke("surface_reload", { request: { instance } }),
    onEvent: (listener) =>
      listen<SurfaceEvent>(SURFACE_EVENT, (event) => {
        listener(event.payload);
      }),
  };
}

/** Implements the app-shell host capabilities with Tauri APIs and commands. */
export class TauriPlatformAdapter implements PlatformAdapter {
  private selectionInProgress = false;

  readonly windowControls: WindowControlsCapability =
    createTauriWindowControls();

  readonly locationActions: LocationActionsCapability =
    createTauriLocationActions();

  readonly surfaces: SurfaceCapability = createTauriSurfaces();

  readonly updates: DesktopUpdateCapability = createTauriUpdates();

  readonly pluginMarketplace: PluginMarketplaceCapability =
    createTauriPluginMarketplace();

  readonly diagnosticLogs: DiagnosticLogsCapability = {
    downloadToday: () => this.downloadTodayLog(),
  };

  readonly worktreeStorage = {
    getRoot: async (): Promise<string> => {
      const config = await invoke<{ worktreeRoot: string }>(
        "get_worktree_root",
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

  /** Opens the native save dialog and hands the chosen path back to the caller. */
  async selectSavePath(options: SelectSavePathOptions): Promise<string | null> {
    if (this.selectionInProgress) {
      throw new PathSelectionInProgressError();
    }

    this.selectionInProgress = true;
    try {
      return await save({ defaultPath: options.defaultFileName });
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

  /** Saves the active daily diagnostic log while sharing the adapter's native-dialog lock. */
  private async downloadTodayLog(): Promise<boolean> {
    if (this.selectionInProgress) {
      throw new PathSelectionInProgressError();
    }

    this.selectionInProgress = true;
    try {
      const now = new Date();
      const year = now.getFullYear();
      const month = String(now.getMonth() + 1).padStart(2, "0");
      const day = String(now.getDate()).padStart(2, "0");
      const destination = await save({
        defaultPath: `ora-logs-${year}-${month}-${day}.log`,
        filters: [{ name: "Log", extensions: ["log"] }],
      });
      if (destination === null) return false;
      await invoke("download_today_log", { request: { destination } });
      return true;
    } finally {
      this.selectionInProgress = false;
    }
  }

  /** Hands an http(s)/mailto URL to the host browser instead of a new webview. */
  async openExternalUrl(url: string): Promise<void> {
    await invoke("open_external_url", { request: { url } });
  }
}

/** Creates the Desktop host adapter without runtime platform auto-detection. */
export function createTauriPlatformAdapter(): TauriPlatformAdapter {
  return new TauriPlatformAdapter();
}
