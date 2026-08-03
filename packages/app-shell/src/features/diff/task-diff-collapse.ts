import type { ChangeData, HunkData } from "react-diff-view";

const CONTEXT_LINE_COUNT = 3;
const MIN_COLLAPSED_LINE_COUNT = 4;

export type DiffRenderSegment =
  | {
      kind: "hunk";
      key: string;
      hunk: HunkData;
    }
  | {
      kind: "collapsed";
      key: string;
      lineCount: number;
    };

interface CollapsedRange {
  start: number;
  end: number;
  key: string;
}

/**
 * Splits complete-context hunks into visible change neighborhoods and expandable
 * unchanged blocks while preserving the parser's original change objects.
 */
export function buildCollapsedDiffSegments(
  hunks: HunkData[],
  expandedBlocks: ReadonlySet<string>,
): DiffRenderSegment[] {
  return hunks.flatMap((hunk, hunkIndex) => {
    const collapsedRanges = findCollapsedRanges(hunk, hunkIndex);
    if (collapsedRanges.length === 0) {
      return [{
        kind: "hunk" as const,
        key: `${hunkIndex}:complete`,
        hunk,
      }];
    }

    const segments: DiffRenderSegment[] = [];
    let cursor = 0;
    collapsedRanges.forEach((range) => {
      if (cursor < range.start) {
        segments.push(createHunkSegment(hunk, hunkIndex, cursor, range.start));
      }
      if (expandedBlocks.has(range.key)) {
        segments.push(createHunkSegment(hunk, hunkIndex, range.start, range.end));
      } else {
        segments.push({
          kind: "collapsed",
          key: range.key,
          lineCount: range.end - range.start,
        });
      }
      cursor = range.end;
    });

    if (cursor < hunk.changes.length) {
      segments.push(createHunkSegment(hunk, hunkIndex, cursor, hunk.changes.length));
    }
    return segments;
  });
}

/** Finds the middle of long normal-line runs while retaining nearby review context. */
function findCollapsedRanges(hunk: HunkData, hunkIndex: number): CollapsedRange[] {
  const ranges: CollapsedRange[] = [];
  let cursor = 0;

  while (cursor < hunk.changes.length) {
    if (hunk.changes[cursor]?.type !== "normal") {
      cursor += 1;
      continue;
    }

    const runStart = cursor;
    while (cursor < hunk.changes.length && hunk.changes[cursor]?.type === "normal") {
      cursor += 1;
    }
    const runEnd = cursor;
    const hiddenStart = runStart + (runStart > 0 ? CONTEXT_LINE_COUNT : 0);
    const hiddenEnd = runEnd - (runEnd < hunk.changes.length ? CONTEXT_LINE_COUNT : 0);
    if (hiddenEnd - hiddenStart < MIN_COLLAPSED_LINE_COUNT) continue;

    ranges.push({
      start: hiddenStart,
      end: hiddenEnd,
      key: `${hunkIndex}:${hunk.content}:${hiddenStart}-${hiddenEnd}`,
    });
  }

  return ranges;
}

/** Rebuilds hunk metadata for one visible slice without cloning its change records. */
function createHunkSegment(
  hunk: HunkData,
  hunkIndex: number,
  start: number,
  end: number,
): DiffRenderSegment {
  const changes = hunk.changes.slice(start, end);
  const precedingChanges = hunk.changes.slice(0, start);
  const oldStart = hunk.oldStart + countSideLines(precedingChanges, "old");
  const newStart = hunk.newStart + countSideLines(precedingChanges, "new");
  const oldLines = countSideLines(changes, "old");
  const newLines = countSideLines(changes, "new");

  return {
    kind: "hunk",
    key: `${hunkIndex}:${start}-${end}`,
    hunk: {
      ...hunk,
      content: `@@ -${oldStart},${oldLines} +${newStart},${newLines} @@`,
      oldStart,
      newStart,
      oldLines,
      newLines,
      changes,
    },
  };
}

/** Counts the lines represented on one side of a change slice. */
function countSideLines(changes: ChangeData[], side: "old" | "new"): number {
  return changes.filter((change) =>
    side === "old" ? change.type !== "insert" : change.type !== "delete"
  ).length;
}
