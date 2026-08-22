import { create } from "zustand";
import { persist } from "zustand/middleware";
import {
  DEFAULT_REVIEW_WIDTH,
  MAX_REVIEW_WIDTH,
  MIN_REVIEW_WIDTH,
} from "../../features/workspace/workspace-review-layout-utils";
import { pathsMatchForWorkspace } from "../../lib/workspace-path";
import { createDebouncedJSONStorage } from "./debounced-json-storage";

export const REVIEW_STORAGE_KEY = "ora.review.v1";

export type ReviewPanelKind = "changes" | "files";

/** Last previewed file while the review panel was open for one checkout scope. */
export interface ReviewFilePersist {
  path: string;
  line?: number;
  column?: number;
}

export interface ReviewContextPersist {
  open: boolean;
  panel: ReviewPanelKind;
  width: number;
  file?: ReviewFilePersist;
}

interface ReviewState {
  byContext: Record<string, ReviewContextPersist>;
  /** Merges one checkout-scoped review snapshot onto disk. */
  upsertContext: (
    contextKey: string,
    patch: Partial<ReviewContextPersist>,
  ) => void;
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
  return {
    path: record.path,
    ...(typeof line === "number" && Number.isFinite(line) ? { line } : {}),
    ...(typeof column === "number" && Number.isFinite(column)
      ? { column }
      : {}),
  };
}

/** Maps an untrusted disk entry onto the review fields the layout owns. */
export function sanitizeReviewContextPersist(
  value: unknown,
): ReviewContextPersist {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return { open: false, panel: "files", width: DEFAULT_REVIEW_WIDTH };
  }
  const record = value as Record<string, unknown>;
  return {
    open: record.open === true,
    panel: sanitizePanel(record.panel),
    width: clampReviewWidth(record.width),
    file: sanitizeFile(record.file),
  };
}

function sanitizeByContext(
  value: unknown,
): Record<string, ReviewContextPersist> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return {};
  }
  const next: Record<string, ReviewContextPersist> = {};
  for (const [key, entry] of Object.entries(value)) {
    if (typeof key !== "string" || key.length === 0) continue;
    next[key] = sanitizeReviewContextPersist(entry);
  }
  return next;
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

/** Builds the file slice written while the review panel is open. */
export function buildReviewFilePersist(input: {
  open: boolean;
  panel: ReviewPanelKind;
  reviewFilePath?: string;
  fileRequest?: { path: string; line?: number };
  workspaceFileRequest?: { path: string; line?: number; column?: number };
}): ReviewFilePersist | undefined {
  const { open, panel, reviewFilePath, fileRequest, workspaceFileRequest } =
    input;
  if (!open || reviewFilePath === undefined) return undefined;
  return {
    path: reviewFilePath,
    ...(panel === "changes" &&
    fileRequest?.line !== undefined &&
    pathsMatchForWorkspace(fileRequest.path, reviewFilePath)
      ? { line: fileRequest.line }
      : {}),
    ...(panel === "files" &&
    workspaceFileRequest?.line !== undefined &&
    pathsMatchForWorkspace(workspaceFileRequest.path, reviewFilePath)
      ? {
          line: workspaceFileRequest.line,
          column: workspaceFileRequest.column,
        }
      : {}),
  };
}

/**
 * Persists the right review rail per checkout scope: open/tab/width and, when
 * the panel was open, the last previewed file path.
 */
export const useReviewStore = create<ReviewState>()(
  persist(
    (set) => ({
      byContext: {},
      upsertContext: (contextKey, patch) =>
        set((state) => {
          const hasCurrent = contextKey in state.byContext;
          const current =
            state.byContext[contextKey] ??
            sanitizeReviewContextPersist(undefined);
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
            file: patch.file !== undefined ? patch.file : current.file,
          };
          if (
            hasCurrent &&
            current.open === next.open &&
            current.panel === next.panel &&
            current.width === next.width &&
            current.file?.path === next.file?.path &&
            current.file?.line === next.file?.line &&
            current.file?.column === next.file?.column
          ) {
            return state;
          }
          return {
            byContext: { ...state.byContext, [contextKey]: next },
          };
        }),
    }),
    {
      name: REVIEW_STORAGE_KEY,
      storage: createDebouncedJSONStorage(),
      partialize: (state) => ({ byContext: state.byContext }),
      merge: (persisted, current) => {
        const slice =
          typeof persisted === "object" && persisted !== null
            ? (persisted as { byContext?: unknown })
            : undefined;
        const disk = sanitizeByContext(slice?.byContext);
        return {
          ...current,
          byContext: { ...disk, ...current.byContext },
        };
      },
    },
  ),
);
