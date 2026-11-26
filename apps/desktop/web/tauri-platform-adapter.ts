import { onCloudConnectionChanged } from "./cloud-connection.generated";
import { onMarketplaceAutoSyncChanged } from "./marketplace-sync.generated";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";
import { normalizeInvokeError } from "./tauri-transport";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import {
  type CloudConnectionCapability,
  type DesktopNotificationOptions,
  type DesktopNotificationsCapability,
  type DesktopUpdateCapability,
  type DesktopUpdateStatus,
  type DiagnosticLogsCapability,
  type InstallationCapability,
  type DownloadAction,
  type LocationActionsCapability,
  type LocationTarget,
  type OsDroppedTextFile,
  PathSelectionInProgressError,
  type PlatformAdapter,
  type PluginInstallProgress,
  type PluginMarketplaceCapability,
  type ResolveDownloadOutcome,
  type SaveTextFileOptions,
  type ScheduledArtifactsCapability,
  type SelectPathOptions,
  type SelectSavePathOptions,
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
  if (os === null) {
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
    // Decoded so the shell can tell a replaced release from a failed installation by its code.
    install: (version) =>
      invoke<void>("install_desktop_update", { version }).catch(
        (error: unknown) => {
          throw normalizeInvokeError(error);
        },
      ),
    check: () => invoke("check_desktop_update"),
    onStatus: (listener) =>
      listen<DesktopUpdateStatus>(DESKTOP_UPDATE_EVENT, (event) => {
        listener(event.payload);
      }),
  };
}

/** Wires marketplace package transfer progress and automatic refreshes into the settings UI. */
function createTauriPluginMarketplace(): PluginMarketplaceCapability {
  return {
    onInstallProgress: (listener) =>
      listen<PluginInstallProgress>(PLUGIN_INSTALL_PROGRESS_EVENT, (event) => {
        listener(event.payload);
      }),
    onAutoSyncChanged: onMarketplaceAutoSyncChanged,
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

/** Wires the native OS notification plugin into the shared shell. */
function createTauriDesktopNotifications(): DesktopNotificationsCapability {
  return {
    show: async (options: DesktopNotificationOptions): Promise<boolean> => {
      let granted = await isPermissionGranted();
      if (!granted) {
        granted = (await requestPermission()) === "granted";
      }
      if (!granted) {
        return false;
      }
      sendNotification({ title: options.title, body: options.body });
      return true;
    },
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
  readonly installation: InstallationCapability = {
    isNew: () => invoke<boolean>("is_new_installation"),
  };

  readonly pluginMarketplace: PluginMarketplaceCapability =
    createTauriPluginMarketplace();

  readonly cloudConnection: CloudConnectionCapability = {
    onChanged: onCloudConnectionChanged,
  };

  readonly desktopNotifications: DesktopNotificationsCapability =
    createTauriDesktopNotifications();

  readonly diagnosticLogs: DiagnosticLogsCapability = {
    downloadToday: () => this.downloadTodayLog(),
    downloadPluginLog: (pluginId, fileStem) =>
      this.downloadPluginLog(pluginId, fileStem),
  };

  readonly scheduledArtifacts: ScheduledArtifactsCapability = {
    download: async (runId, relativePath, destination) => {
      await invoke("download_scheduled_run_artifact", {
        request: { runId, relativePath, destination },
      });
    },
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
  private downloadTodayLog(): Promise<boolean> {
    return this.saveLogThroughDialog(
      `ora-logs-${localDateStamp()}.log`,
      (destination) =>
        invoke("download_today_log", { request: { destination } }),
    );
  }

  /** Saves one plugin's host-owned log while sharing the adapter's native-dialog lock. */
  private downloadPluginLog(
    pluginId: string,
    fileStem: string,
  ): Promise<boolean> {
    const stem = fileStem.replace(/[^A-Za-z0-9._-]+/g, "-") || "plugin";
    return this.saveLogThroughDialog(
      `ora-plugin-${stem}-${localDateStamp()}.log`,
      (destination) =>
        invoke("download_plugin_log", { request: { pluginId, destination } }),
    );
  }

  /** Runs one native save dialog and hands the chosen destination to the host copy command. */
  private async saveLogThroughDialog(
    defaultPath: string,
    copy: (destination: string) => Promise<unknown>,
  ): Promise<boolean> {
    if (this.selectionInProgress) {
      throw new PathSelectionInProgressError();
    }

    this.selectionInProgress = true;
    try {
      const destination = await save({
        defaultPath,
        filters: [{ name: "Log", extensions: ["log"] }],
      });
      if (destination === null) return false;
      await copy(destination);
      return true;
    } finally {
      this.selectionInProgress = false;
    }
  }

  /** Hands an http(s)/mailto URL to the host browser instead of a new webview. */
  async openExternalUrl(url: string): Promise<void> {
    await invoke("open_external_url", { request: { url } });
  }

  /**
   * Forwards native OS file drops. WebView2 leaves HTML5 `dataTransfer.files` empty,
   * so the import picker must use this event instead of the browser drop payload.
   */
  subscribeOsFileDrop(
    listener: (paths: readonly string[]) => void,
  ): () => void {
    if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
      return () => undefined;
    }
    let active = true;
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      if (active && event.payload.type === "drop") {
        listener(event.payload.paths);
      }
    });
    return () => {
      active = false;
      void unlisten.then((stop) => stop());
    };
  }

  /** Loads a dropped path through the desktop command that owns filesystem reads. */
  async readOsTextFile(path: string): Promise<OsDroppedTextFile> {
    return invoke<OsDroppedTextFile>("read_workflow_import", {
      request: { path },
    });
  }
}

/** Renders the local calendar date as `YYYY-MM-DD` for suggested log file names. */
function localDateStamp(): string {
  const now = new Date();
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

/** Creates the Desktop host adapter without runtime platform auto-detection. */
export function createTauriPlatformAdapter(): TauriPlatformAdapter {
  return new TauriPlatformAdapter();
}
