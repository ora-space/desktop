import { create } from "zustand";
import {
  DEFAULT_REVIEW_WIDTH,
  MAX_REVIEW_WIDTH,
  MIN_REVIEW_WIDTH,
} from "./review-layout";
import { pathsMatchForWorkspace } from "../../lib/workspace-path";

// The review rail no longer survives a restart (every launch opens it closed).
// Wipe the key older versions wrote so a stale snapshot never lingers unread.
const LEGACY_REVIEW_STORAGE_KEY = "ora.review.v1";
if (typeof window !== "undefined") {
  // Storage can throw (blocked/disabled); a wipe failure must never block the
  // module loading and take the store down with it on startup.
  try {
    window.localStorage.removeItem(LEGACY_REVIEW_STORAGE_KEY);
  } catch {
    // Ignore: the stale key simply stays until a later run wipes it.
  }
}

export type ReviewPanelKind = "changes" | "files";

/** Last previewed file while the review panel was open for one checkout scope. */
export interface ReviewFilePersist {
  path: string;
  line?: number;
  column?: number;
  endLine?: number;
  side?: "old" | "new";
}

/**
 * Last previewed file per panel. Keyed by panel because the two tabs browse
 * different path spaces: Changes only knows paths present in the diff, Files
 * knows the whole checkout. A single shared slot lets a Changes-only path (a
 * deleted or renamed file) be replayed into the Files panel after a restart.
 */
export type ReviewFilesPersist = Partial<
  Record<ReviewPanelKind, ReviewFilePersist>
>;

export interface ReviewContextPersist {
  open: boolean;
  panel: ReviewPanelKind;
  width: number;
  files: ReviewFilesPersist;
}

interface ReviewState {
  byContext: Record<string, ReviewContextPersist>;
  /**
   * Merges one checkout-scoped review snapshot into memory. `files` is merged
   * per panel, so writing the Changes preview never disturbs the Files one.
   */
  upsertContext: (
    contextKey: string,
    patch: Partial<ReviewContextPersist>,
  ) => void;
  /**
   * Drops scopes whose project or task no longer exists so deleted rows do not
   * accumulate for the rest of the run (mirrors `pruneTreeExpansion` in the UI store).
   */
  pruneContexts: (
    projectIds: readonly string[],
    taskIds: readonly string[],
  ) => void;
}

/** True when two per-panel file slices carry the same path/line/column. */
function reviewFilesEqual(
  left: ReviewFilesPersist,
  right: ReviewFilesPersist,
): boolean {
  const panels: readonly ReviewPanelKind[] = ["changes", "files"];
  return panels.every((panel) => {
    const a = left[panel];
    const b = right[panel];
    return (
      a?.path === b?.path &&
      a?.line === b?.line &&
      a?.column === b?.column &&
      a?.endLine === b?.endLine &&
      a?.side === b?.side
    );
  });
}

/** Clamps a persisted review width into the live drag range. */
export function clampReviewWidth(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return DEFAULT_REVIEW_WIDTH;
  }
  return Math.min(
    MAX_REVIEW_WIDTH,
    Math.max(MIN_REVIEW_WIDTH, Math.round(value)),
  );
}

function sanitizePanel(value: unknown): ReviewPanelKind {
  return value === "changes" ? "changes" : "files";
}

function sanitizeFile(value: unknown): ReviewFilePersist | undefined {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return undefined;
  }
  const record = value as Record<string, unknown>;
  if (typeof record.path !== "string" || record.path.length === 0) {
    return undefined;
  }
  const line = record.line;
  const column = record.column;
  const endLine = record.endLine;
  const side = record.side;
  return {
    path: record.path,
    ...(typeof line === "number" && Number.isFinite(line) ? { line } : {}),
    ...(typeof column === "number" && Number.isFinite(column)
      ? { column }
      : {}),
    ...(typeof endLine === "number" && Number.isFinite(endLine)
      ? { endLine }
      : {}),
    ...(side === "old" || side === "new" ? { side } : {}),
  };
}

/** Keeps only the per-panel entries that survive `sanitizeFile`. */
export function sanitizeFiles(value: unknown): ReviewFilesPersist {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return {};
  }
  const record = value as Record<string, unknown>;
  const changes = sanitizeFile(record.changes);
  const files = sanitizeFile(record.files);
  return {
    ...(changes !== undefined ? { changes } : {}),
    ...(files !== undefined ? { files } : {}),
  };
}

/** Maps an untrusted disk entry onto the review fields the layout owns. */
export function sanitizeReviewContextPersist(
  value: unknown,
): ReviewContextPersist {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return {
      open: false,
      panel: "files",
      width: DEFAULT_REVIEW_WIDTH,
      files: {},
    };
  }
  const record = value as Record<string, unknown>;
  return {
    open: record.open === true,
    panel: sanitizePanel(record.panel),
    width: clampReviewWidth(record.width),
    files: sanitizeFiles(record.files),
  };
}

/** Stable key for one project checkout or task worktree review scope. */
export function reviewContextKey(context: {
  kind: "none" | "project" | "task";
  projectId?: string;
  taskId?: string;
}): string | null {
  if (context.kind === "none") return null;
  if (context.kind === "project") return `project:${context.projectId}`;
  return `task:${context.taskId}`;
}

/**
 * Builds the file slice written while the review panel is open.
 *
 * The caller stores the result under the *live* panel, so a Changes path can
 * never end up as the Files panel's restored selection.
 */
export function buildReviewFilePersist(input: {
  open: boolean;
  panel: ReviewPanelKind;
  reviewFilePath?: string;
  fileRequest?: {
    path: string;
    line?: number;
    endLine?: number;
    side?: "old" | "new";
  };
  workspaceFileRequest?: {
    path: string;
    line?: number;
    column?: number;
    endLine?: number;
  };
}): ReviewFilePersist | undefined {
  const { open, panel, reviewFilePath, fileRequest, workspaceFileRequest } =
    input;
  if (!open || reviewFilePath === undefined) return undefined;
  return {
    path: reviewFilePath,
    ...(panel === "changes" &&
    fileRequest?.line !== undefined &&
    pathsMatchForWorkspace(fileRequest.path, reviewFilePath)
      ? {
          line: fileRequest.line,
          ...(fileRequest.endLine !== undefined
            ? { endLine: fileRequest.endLine }
            : {}),
          ...(fileRequest.side !== undefined ? { side: fileRequest.side } : {}),
        }
      : {}),
    ...(panel === "files" &&
    workspaceFileRequest?.line !== undefined &&
    pathsMatchForWorkspace(workspaceFileRequest.path, reviewFilePath)
      ? {
          line: workspaceFileRequest.line,
          column: workspaceFileRequest.column,
          ...(workspaceFileRequest.endLine !== undefined
            ? { endLine: workspaceFileRequest.endLine }
            : {}),
        }
      : {}),
  };
}

/**
 * Remembers the right review rail per checkout scope for the rest of the app
 * run: open/tab/width and, when the panel was open, the last previewed file
 * path. Nothing is written to disk, so a restart always opens the rail closed
 * while switching between scopes in-session still re-applies each scope's state.
 */
export const useReviewStore = create<ReviewState>()((set) => ({
  byContext: {},
  upsertContext: (contextKey, patch) =>
    set((state) => {
      const hasCurrent = contextKey in state.byContext;
      const current =
        state.byContext[contextKey] ?? sanitizeReviewContextPersist(undefined);
      const next: ReviewContextPersist = {
        open: patch.open ?? current.open,
        panel:
          patch.panel !== undefined
            ? sanitizePanel(patch.panel)
            : current.panel,
        width:
          patch.width !== undefined
            ? clampReviewWidth(patch.width)
            : current.width,
        // Sanitize here too: `upsertContext` is the single write door, so
        // everything in `byContext` stays sanitized by construction rather
        // than by convention at each call site.
        files:
          patch.files !== undefined
            ? { ...current.files, ...sanitizeFiles(patch.files) }
            : current.files,
      };
      if (
        hasCurrent &&
        current.open === next.open &&
        current.panel === next.panel &&
        current.width === next.width &&
        reviewFilesEqual(current.files, next.files)
      ) {
        return state;
      }
      return {
        byContext: { ...state.byContext, [contextKey]: next },
      };
    }),
  pruneContexts: (projectIds, taskIds) =>
    set((state) => {
      const live = new Set([
        ...projectIds.map((id) => `project:${id}`),
        ...taskIds.map((id) => `task:${id}`),
      ]);
      const entries = Object.entries(state.byContext).filter(([key]) =>
        live.has(key),
      );
      if (entries.length === Object.keys(state.byContext).length) {
        return state;
      }
      return { byContext: Object.fromEntries(entries) };
    }),
}));
