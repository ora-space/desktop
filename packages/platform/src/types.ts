export type PathSelectionKind = "file" | "directory";

export interface SelectPathOptions {
  kind: PathSelectionKind;
  initialPath?: string;
}

/** Abstracts one single-path selection interaction across Web and Tauri hosts. */
export interface PlatformAdapter {
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
