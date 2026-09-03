export type PathSelectionKind = "file" | "directory";

export interface SelectPathOptions {
  kind: PathSelectionKind;
  initialPath?: string;
}

/** Defines one user-initiated text-file export without exposing host-specific dialogs. */
export interface SaveTextFileOptions {
  defaultFileName: string;
  content: string;
}

/** Defines one native save dialog whose chosen path the caller consumes itself. */
export interface SelectSavePathOptions {
  defaultFileName: string;
}

/** Reads and updates the Desktop worktree root used for new task worktrees. */
export interface WorktreeStorageCapability {
  getRoot(): Promise<string>;
  setRoot(path: string): Promise<void>;
}

/** The host operating system, as far as the window chrome needs to care. */
export type WindowManagerOs = "windows" | "macos" | "linux";

/**
 * Whether this host wants the app to paint its own window controls.
 *
 * macOS (which keeps its native traffic lights) reports `none`, so the shell
 * renders no controls at all. A frameless Windows/Linux window reports
 * `overlay` and hands back the imperative window commands the custom title bar
 * drives.
 */
export type WindowControlsCapability =
  | { kind: "none" }
  | {
      kind: "overlay";
      os: WindowManagerOs;
      minimize(): Promise<void>;
      toggleMaximize(): Promise<void>;
      close(): Promise<void>;
      isMaximized(): Promise<boolean>;
      /**
       * Observes maximize-state changes so the maximize/restore glyph can follow
       * the window. Returns an unsubscribe function.
       */
      subscribeMaximized(listener: (maximized: boolean) => void): () => void;
    };

/** The host application a resolved location can be handed off to. */
export type LocationTarget = "explorer" | "terminal" | "vscode";

/**
 * Hands an absolute path off to a file manager, terminal, or VS Code on the host OS.
 *
 * Desktop exposes the calls the split button drives - resolving either a
 * Workspace or task directory, then opening it in the chosen target.
 */
export interface LocationActionsCapability {
  /** Resolves the absolute working directory backing one isolated worktree task. */
  resolveTaskCwd(taskId: string): Promise<string>;
  /** Resolves the absolute local directory backing one Workspace. */
  resolveWorkspaceCwd(workspaceId: string): Promise<string>;
  /** Opens one absolute path in the chosen host application. */
  open(target: LocationTarget, path: string): Promise<void>;
}

/**
 * Explains why a release cannot be installed by the updater itself. Mirrors the Rust
 * `ManualUpdateReason` in `apps/desktop/src-tauri/src/update.rs`.
 */
export type ManualUpdateReason = "system_package" | "unpackaged_binary";

/** Describes the native Desktop updater state shown by the shell's version control. */
export type DesktopUpdateStatus =
  | { kind: "current" }
  | { kind: "checking" }
  | {
      kind: "downloading";
      version: string;
      downloaded: number;
      total: number | null;
    }
  | { kind: "ready"; version: string }
  | { kind: "manual_update"; version: string; reason: ManualUpdateReason }
  | { kind: "installing"; version: string }
  | { kind: "failed"; message: string };

/** Exposes update status and installation without coupling shared UI to Tauri IPC. */
export interface DesktopUpdateCapability {
  getStatus(): Promise<DesktopUpdateStatus>;
  install(): Promise<void>;
  /** Runs an update check on demand, outside the scheduled delayed and cron checks. */
  check(): Promise<void>;
  onStatus(
    listener: (status: DesktopUpdateStatus) => void,
  ): Promise<() => void>;
}

/** Byte-level progress for one marketplace package download. */
export interface PluginInstallProgress {
  pluginId: string;
  downloaded: number;
  total: number | null;
}

/** Exposes native marketplace transfer events without coupling shared UI to Tauri. */
export interface PluginMarketplaceCapability {
  onInstallProgress(
    listener: (progress: PluginInstallProgress) => void,
  ): Promise<() => void>;
}

/** Where a plugin surface renders: docked into the right panel or in its own native window. */
export type SurfaceTarget = "embedded" | "windowed";

/** Lifecycle of one native surface instance, mirrored from the backend registry. */
export type SurfaceState =
  "opening" | "open" | "migrating" | "closing" | "failed";

/** One live plugin surface owned by the host runtime. */
/** Which kind of surface a record hosts; decides the entry look and the download behavior. */
export type SurfaceKind = "workbench" | "webview";

/** One live plugin surface owned by the host runtime. */
export interface SurfaceRecord {
  instance: number;
  pluginId: string;
  kind: SurfaceKind;
  title: string;
  target: SurfaceTarget;
  state: SurfaceState;
}

/** Lifecycle and download notifications emitted by the host for every surface instance. */
export type SurfaceEvent =
  | {
      type: "opened";
      instance: number;
      pluginId: string;
      kind: SurfaceKind;
      target: SurfaceTarget;
      title: string;
    }
  | { type: "migrated"; instance: number; target: SurfaceTarget }
  | { type: "migrateFailed"; instance: number; reason: string }
  | { type: "failed"; instance: number; reason: string }
  | { type: "closed"; instance: number }
  | {
      type: "downloadStarted";
      instance: number;
      pluginId: string;
      downloadId: number;
      fileName: string;
    }
  | {
      /** A prompt-disposition download landed; the user must pick one of `actions`. */
      type: "downloadChoice";
      instance: number;
      pluginId: string;
      downloadId: number;
      pageOrigin: string;
      fileName: string;
      sizeBytes: number;
      actions: DownloadAction[];
    }
  | {
      type: "downloadCompleted";
      instance: number;
      pluginId: string;
      downloadId: number;
      fileName: string;
      action: string;
      /** For a completed `import_skill`: the prepared import session to open for review. */
      importSessionId: string | null;
    }
  | {
      type: "downloadFailed";
      instance: number;
      pluginId: string;
      downloadId: number;
      fileName: string;
      reason: string;
    };

/** The closed set of host download actions a webview-plugin download may resolve to. */
export type DownloadAction = "import_skill" | "save_as";

/** The result of resolving a webview-plugin download. */
export interface ResolveDownloadOutcome {
  action: string;
  importSessionId: string | null;
}

/** The placeholder rectangle in CSS pixels plus the device scale the native layer needs. */
export interface SurfaceBounds {
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
}

/** Identifies the single surface an installed workbench or webview plugin contributes. */
export interface SurfaceOpenTarget {
  pluginId: string;
}

/** Drives native plugin surfaces (embedded child webviews or standalone windows). */
export interface SurfaceCapability {
  capabilities(): Promise<{ embedded: boolean }>;
  list(): Promise<SurfaceRecord[]>;
  open(target: SurfaceOpenTarget, mount: SurfaceTarget): Promise<SurfaceRecord>;
  close(instance: number): Promise<void>;
  setBounds(instance: number, bounds: SurfaceBounds): Promise<void>;
  setVisible(instance: number, visible: boolean): Promise<void>;
  popout(instance: number): Promise<void>;
  dock(instance: number): Promise<void>;
  reload(instance: number): Promise<void>;
  /** Runs a host action for a prompt-disposition webview download. */
  resolveDownload(
    downloadId: number,
    action: DownloadAction,
    destination?: string,
  ): Promise<ResolveDownloadOutcome>;
  /** Discards a prompt-disposition webview download the user dismissed. */
  discardDownload(downloadId: number): Promise<void>;
  onEvent(listener: (event: SurfaceEvent) => void): Promise<() => void>;
}

/** Lets the shell export the host's current diagnostic log without learning its private path. */
export interface DiagnosticLogsCapability {
  /** Opens a host save flow and reports whether the user completed the download. */
  downloadToday(): Promise<boolean>;
}

/** Collects the host capabilities consumed by the shared application shell. */
export interface PlatformAdapter {
  readonly worktreeStorage: WorktreeStorageCapability;
  readonly windowControls: WindowControlsCapability;
  readonly locationActions: LocationActionsCapability;
  readonly surfaces: SurfaceCapability;
  readonly updates?: DesktopUpdateCapability;
  readonly pluginMarketplace?: PluginMarketplaceCapability;
  readonly diagnosticLogs?: DiagnosticLogsCapability;
  selectPath(options: SelectPathOptions): Promise<string | null>;
  /** Opens the native save dialog and returns the chosen path, or null when dismissed. */
  selectSavePath(options: SelectSavePathOptions): Promise<string | null>;
  saveTextFile(options: SaveTextFileOptions): Promise<boolean>;
  /**
   * Opens an http(s) or mailto URL in the host browser. Prompt-box links call
   * this so Desktop is not stuck with a webview `window.open` that never leaves
   * the app.
   */
  openExternalUrl(url: string): Promise<void>;
}

/** Reports a caller bug that attempts to open two selectors on one adapter concurrently. */
export class PathSelectionInProgressError extends Error {
  constructor() {
    super("a path selection request is already in progress");
    this.name = "PathSelectionInProgressError";
  }
}
