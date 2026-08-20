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
 * Desktop exposes the two calls the split button drives - resolving the git
 * worktree directory that backs a task, then opening it in the chosen target.
 */
export interface LocationActionsCapability {
  /** Resolves the absolute working directory (git worktree root) backing one task. */
  resolveTaskCwd(taskId: string): Promise<string>;
  /** Opens one absolute path in the chosen host application. */
  open(target: LocationTarget, path: string): Promise<void>;
}

/** Where a plugin surface renders: docked into the right panel or in its own native window. */
export type SurfaceTarget = "embedded" | "windowed";

/** Lifecycle of one native surface instance, mirrored from the backend registry. */
export type SurfaceState =
  "opening" | "open" | "migrating" | "closing" | "failed";

/** One live plugin surface owned by the host runtime. */
export interface SurfaceRecord {
  instance: number;
  pluginId: string;
  surfaceId: string;
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
      surfaceId: string;
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
      fileName: string;
    }
  | {
      type: "downloadCompleted";
      instance: number;
      pluginId: string;
      fileName: string;
      path: string;
    }
  | {
      type: "downloadFailed";
      instance: number;
      pluginId: string;
      fileName: string;
      reason: string;
    };

/** The placeholder rectangle in CSS pixels plus the device scale the native layer needs. */
export interface SurfaceBounds {
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
}

/** Identifies one surface declared by an installed ui plugin. */
export interface SurfaceDefinitionId {
  pluginId: string;
  surfaceId: string;
}

/** Drives native plugin surfaces (embedded child webviews or standalone windows). */
export interface SurfaceCapability {
  capabilities(): Promise<{ embedded: boolean }>;
  list(): Promise<SurfaceRecord[]>;
  open(
    definition: SurfaceDefinitionId,
    target: SurfaceTarget,
  ): Promise<SurfaceRecord>;
  close(instance: number): Promise<void>;
  setBounds(instance: number, bounds: SurfaceBounds): Promise<void>;
  setVisible(instance: number, visible: boolean): Promise<void>;
  popout(instance: number): Promise<void>;
  dock(instance: number): Promise<void>;
  reload(instance: number): Promise<void>;
  onEvent(listener: (event: SurfaceEvent) => void): Promise<() => void>;
}

/** Collects the host capabilities consumed by the shared application shell. */
export interface PlatformAdapter {
  readonly worktreeStorage: WorktreeStorageCapability;
  readonly windowControls: WindowControlsCapability;
  readonly locationActions: LocationActionsCapability;
  readonly surfaces: SurfaceCapability;
  selectPath(options: SelectPathOptions): Promise<string | null>;
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
