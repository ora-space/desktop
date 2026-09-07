import { parseDiff, type FileData } from "react-diff-view";

export interface DiffStats {
  additions: number;
  deletions: number;
}

/**
 * Patches above this many characters parse in two steps: the first render
 * returns an empty list, a post-paint macro-task then splits/parses. Session
 * switches remount this panel inside their own commit, so a multi-megabyte
 * cached patch would otherwise block that paint. Mount-only — a refetch of
 * the same patch never re-defers.
 */
export const DEFER_PARSE_PATCH_CHARS = 512_000;

/**
 * Splits a unified patch back into per-file segments. Boundary `diff --git`
 * lines are emitted by git for every changed file, so one cheap linear scan
 * locates all file slices without tokenizing them. Used both to defer parsing
 * until after first paint and to cache parsed `FileData` per segment so a
 * live-sync invalidation does not invalidate every memoized file render.
 */
export function splitPatchSegments(patch: string): string[] {
  if (patch.trim().length === 0) return [];
  const lines = patch.split("\n");
  const segments: string[] = [];
  let cursor = -1;
  for (let index = 0; index < lines.length; index += 1) {
    if (lines[index]!.startsWith("diff --git ")) {
      if (cursor >= 0) segments.push(lines.slice(cursor, index).join("\n"));
      cursor = index;
    }
  }
  // Fallback: a patch without the git header (foreign format) parses as one
  // segment; this keeps the split helper total instead of dropping content.
  if (cursor === -1) return [patch];
  segments.push(lines.slice(cursor).join("\n"));
  return segments;
}

/** Parses a single-file patch segment (always exactly one `FileData`). */
export function parseTaskDiffSegment(segment: string): FileData | undefined {
  return parseTaskDiffPatch(segment)[0];
}

/** Treats an empty backend snapshot as no files instead of a synthetic blank diff entry. */
export function parseTaskDiffPatch(patch: string): FileData[] {
  return patch.trim().length === 0 ? [] : parseDiff(patch);
}

/** Counts inserted and deleted lines across parsed patch files. */
export function countChanges(files: FileData[]): DiffStats {
  return files.reduce(
    (total, file) =>
      file.hunks.reduce(
        (fileTotal, hunk) =>
          hunk.changes.reduce(
            (hunkTotal, change) => ({
              additions:
                hunkTotal.additions + (change.type === "insert" ? 1 : 0),
              deletions:
                hunkTotal.deletions + (change.type === "delete" ? 1 : 0),
            }),
            fileTotal,
          ),
        total,
      ),
    { additions: 0, deletions: 0 },
  );
}
