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

/** A native marketplace integration exposed by Ora Desktop. */
export type SkillMarketplaceProvider = "skillHub" | "huaweiAgentCenter";

/** Reports the download lifecycle controlled by a provider-specific native marketplace window. */
export type SkillMarketplaceStatus =
  | {
      status: "downloading";
      provider: SkillMarketplaceProvider;
      fileName: string;
    }
  | {
      status: "downloaded";
      provider: SkillMarketplaceProvider;
      fileName: string;
      archivePath: string;
    }
  | {
      status: "failed";
      provider: SkillMarketplaceProvider;
      stage: "download";
      code: string;
      message: string;
    };

/** Opens a provider-specific native WebView and observes its Ora-owned download lifecycle. */
export interface SkillMarketplaceCapability {
  open(provider: SkillMarketplaceProvider): Promise<void>;
  onStatus(
    listener: (status: SkillMarketplaceStatus) => void,
  ): Promise<() => void>;
}

/** Collects the host capabilities consumed by the shared application shell. */
export interface PlatformAdapter {
  readonly worktreeStorage: WorktreeStorageCapability;
  readonly windowControls: WindowControlsCapability;
  readonly locationActions: LocationActionsCapability;
  readonly skillMarketplace: SkillMarketplaceCapability;
  selectPath(options: SelectPathOptions): Promise<string | null>;
  saveTextFile(options: SaveTextFileOptions): Promise<boolean>;
}

/** Reports a caller bug that attempts to open two selectors on one adapter concurrently. */
export class PathSelectionInProgressError extends Error {
  constructor() {
    super("a path selection request is already in progress");
    this.name = "PathSelectionInProgressError";
  }
}
