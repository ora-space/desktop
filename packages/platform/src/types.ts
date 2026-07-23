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

/** Abstracts one single-path selection interaction across Web and Tauri hosts. */
export interface PlatformAdapter {
  readonly worktreeStorage: WorktreeStorageCapability;
  readonly windowControls: WindowControlsCapability;
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
