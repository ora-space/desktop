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

export interface DiffLineTarget {
  change: ChangeData;
  collapsedKey: string | null;
}

/** Returns the old or new source line represented by one parsed change. */
function lineNumberFor(change: ChangeData, side: "old" | "new"): number | null {
  if (change.type === "normal") {
    return side === "old" ? change.oldLineNumber : change.newLineNumber;
  }
  if (change.type === "delete") {
    return side === "old" ? change.lineNumber : null;
  }
  return side === "new" ? change.lineNumber : null;
}

/**
 * Locates every line on `side` in `[startLine, endLine]` and names any
 * collapsed block currently hiding each one, so a chat jump can expand then
 * scroll.
 */
export function findDiffLineTargets(
  hunks: HunkData[],
  startLine: number,
  endLine: number,
  side: "old" | "new" = "new",
): DiffLineTarget[] {
  const start = Math.min(startLine, endLine);
  const end = Math.max(startLine, endLine);
  const targets: DiffLineTarget[] = [];
  for (let hunkIndex = 0; hunkIndex < hunks.length; hunkIndex += 1) {
    const hunk = hunks[hunkIndex]!;
    const ranges = findCollapsedRanges(hunk, hunkIndex);
    for (let index = 0; index < hunk.changes.length; index += 1) {
      const change = hunk.changes[index]!;
      const line = lineNumberFor(change, side);
      if (line === null || line < start || line > end) continue;
      const collapsed = ranges.find(
        (range) => index >= range.start && index < range.end,
      );
      targets.push({ change, collapsedKey: collapsed?.key ?? null });
    }
  }
  return targets;
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
      return [
        {
          kind: "hunk" as const,
          key: `${hunkIndex}:complete`,
          hunk,
        },
      ];
    }

    const segments: DiffRenderSegment[] = [];
    let cursor = 0;
    collapsedRanges.forEach((range) => {
      if (cursor < range.start) {
        segments.push(createHunkSegment(hunk, hunkIndex, cursor, range.start));
      }
      if (expandedBlocks.has(range.key)) {
        segments.push(
          createHunkSegment(hunk, hunkIndex, range.start, range.end),
        );
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
      segments.push(
        createHunkSegment(hunk, hunkIndex, cursor, hunk.changes.length),
      );
    }
    return segments;
  });
}

/** Finds the middle of long normal-line runs while retaining nearby changed context. */
function findCollapsedRanges(
  hunk: HunkData,
  hunkIndex: number,
): CollapsedRange[] {
  const ranges: CollapsedRange[] = [];
  let cursor = 0;

  while (cursor < hunk.changes.length) {
    if (hunk.changes[cursor]?.type !== "normal") {
      cursor += 1;
      continue;
    }

    const runStart = cursor;
    while (
      cursor < hunk.changes.length &&
      hunk.changes[cursor]?.type === "normal"
    ) {
      cursor += 1;
    }
    const runEnd = cursor;
    const hiddenStart = runStart + (runStart > 0 ? CONTEXT_LINE_COUNT : 0);
    const hiddenEnd =
      runEnd - (runEnd < hunk.changes.length ? CONTEXT_LINE_COUNT : 0);
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
  return {
    kind: "hunk",
    key: `${hunkIndex}:${start}-${end}`,
    hunk: sliceHunkData(hunk, start, end),
  };
}

/**
 * Rebuilds one hunk's metadata for a `[start, end)` change slice, recomputing
 * the `@@` header and old/new line ranges so the slice renders standalone as a
 * native `react-diff-view` hunk. Shared by the collapsed-segment builder and
 * the row-windowed body, which splits a huge hunk into fixed-size native
 * hunks.
 */
export function sliceHunkData(
  hunk: HunkData,
  start: number,
  end: number,
): HunkData {
  const changes = hunk.changes.slice(start, end);
  const precedingChanges = hunk.changes.slice(0, start);
  const oldStart = hunk.oldStart + countSideLines(precedingChanges, "old");
  const newStart = hunk.newStart + countSideLines(precedingChanges, "new");
  const oldLines = countSideLines(changes, "old");
  const newLines = countSideLines(changes, "new");

  return {
    ...hunk,
    content: `@@ -${oldStart},${oldLines} +${newStart},${newLines} @@`,
    oldStart,
    newStart,
    oldLines,
    newLines,
    changes,
  };
}

/** Counts the lines represented on one side of a change slice. */
function countSideLines(changes: ChangeData[], side: "old" | "new"): number {
  return changes.filter((change) =>
    side === "old" ? change.type !== "insert" : change.type !== "delete",
  ).length;
}

/**
 * Maximum change rows each row-windowed chunk renders as one native hunk. A
 * chunk is a standalone `react-diff-view` `Hunk`, so a huge file becomes
 * N/ROWS_PER_CHUNK native hunks and only the visible chunk window is mounted.
 */
export const ROWS_PER_CHUNK = 60;

/**
 * One renderable chunk for the row-windowed body: either a native hunk slice,
 * or a folded-unchanged block. Both are what `react-diff-view` renders natively,
 * so every mounted chunk keeps the exact gutter/tint/collapse styling.
 */
export type DiffRenderChunk =
  | { kind: "hunk"; key: string; hunk: HunkData }
  | { kind: "collapsed"; key: string; lineCount: number };

/**
 * Splits collapsed-render segments into fixed-size native hunk chunks. A hunk
 * larger than `ROWS_PER_CHUNK` is sliced into smaller hunks with recomputed
 * `@@` line ranges (see `sliceHunkData`), so each chunk renders standalone as a
 * native `<Hunk>`; a folded-unchanged segment stays a single chunk. The
 * returned chunks preserve the original change objects, so a jump that resolves
 * to a change can be mapped back to its chunk by reference.
 */
export function buildRenderChunks(
  segments: readonly DiffRenderSegment[],
): DiffRenderChunk[] {
  const chunks: DiffRenderChunk[] = [];
  for (const segment of segments) {
    if (segment.kind === "collapsed") {
      chunks.push({
        kind: "collapsed",
        key: segment.key,
        lineCount: segment.lineCount,
      });
      continue;
    }
    const { hunk } = segment;
    if (hunk.changes.length <= ROWS_PER_CHUNK) {
      chunks.push({ kind: "hunk", key: segment.key, hunk });
      continue;
    }
    for (let start = 0; start < hunk.changes.length; start += ROWS_PER_CHUNK) {
      const end = Math.min(start + ROWS_PER_CHUNK, hunk.changes.length);
      chunks.push({
        kind: "hunk",
        key: `${segment.key}:${start}`,
        hunk: sliceHunkData(hunk, start, end),
      });
    }
  }
  return chunks;
}

/**
 * Finds the chunk index (see `buildRenderChunks`) that holds `change`. Used by
 * the jump-to-line flow to scroll the virtualizer to the right chunk before the
 * highlighted row can be mounted and scrolled into view.
 */
export function chunkIndexForChange(
  chunks: readonly DiffRenderChunk[],
  change: ChangeData,
): number {
  return chunks.findIndex(
    (chunk) => chunk.kind === "hunk" && chunk.hunk.changes.includes(change),
  );
}

/** Approximate height of a chunk so the virtualizer can size the scroll track. */
export function estimateChunkHeight(chunk: DiffRenderChunk): number {
  // 20px per row matches the native `text-xs leading-5` geometry; collapsed
  // blocks and hunk headers are a fixed 32px.
  return chunk.kind === "collapsed"
    ? 32
    : Math.max(1, chunk.hunk.changes.length) * 20;
}
