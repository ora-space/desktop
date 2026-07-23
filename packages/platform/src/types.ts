export type PathSelectionKind = "file" | "directory";

export interface SelectPathOptions {
  kind: PathSelectionKind;
  initialPath?: string;
}

export type WorktreeStorageCapability =
  | { kind: "unsupported" }
  | {
      kind: "configurable";
      getRoot(): Promise<string>;
      setRoot(path: string): Promise<void>;
    };

/** The host operating system, as far as the window chrome needs to care. */
export type WindowManagerOs = "windows" | "macos" | "linux";

/**
 * Whether this host wants the app to paint its own window controls.
 *
 * The Web host and macOS (which keeps its native traffic lights) report `none`,
 * so the shell renders no controls at all. A frameless Windows/Linux window
 * reports `overlay` and hands back the imperative window commands the custom
 * title bar drives.
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
 * This is a desktop-only convenience: the Web host cannot launch native applications,
 * so it reports `unsupported` and the toolbar entry point stays hidden. Desktop hosts
 * report `supported` and expose the two calls the split button drives - resolving the
 * git worktree directory that backs a task, then opening it in the chosen target.
 */
export type LocationActionsCapability =
  | { kind: "unsupported" }
  | {
      kind: "supported";
      /** Resolves the absolute working directory (git worktree root) backing one task. */
      resolveTaskCwd(taskId: string): Promise<string>;
      /** Opens one absolute path in the chosen host application. */
      open(target: LocationTarget, path: string): Promise<void>;
    };

/** Abstracts one single-path selection interaction across Web and Tauri hosts. */
export interface PlatformAdapter {
  readonly worktreeStorage: WorktreeStorageCapability;
  readonly windowControls: WindowControlsCapability;
  readonly locationActions: LocationActionsCapability;
  selectPath(options: SelectPathOptions): Promise<string | null>;
}

export type PlatformLocale = "zh-CN" | "en-US";

/** Reports a caller bug that attempts to open two selectors on one adapter concurrently. */
export class PathSelectionInProgressError extends Error {
  constructor() {
    super("a path selection request is already in progress");
    this.name = "PathSelectionInProgressError";
  }
}
