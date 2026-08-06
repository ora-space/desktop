import { parseDiff, type FileData } from "react-diff-view";

export interface DiffStats {
  additions: number;
  deletions: number;
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
