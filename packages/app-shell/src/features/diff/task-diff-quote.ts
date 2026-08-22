import type { ChangeData, FileData } from "react-diff-view";

export type DiffQuoteSide = "old" | "new";

export interface DiffQuoteAnchor {
  path: string;
  side: DiffQuoteSide;
  line: number;
  changeKey: string;
  content: string;
  changeType: "insert" | "delete" | "normal";
}

/**
 * Whether this gutter cell can open a quote control. New-side owns insert and
 * context rows; old-side only owns pure deletes so removals stay quotable.
 */
export function canQuoteDiffChange(
  change: ChangeData,
  side: DiffQuoteSide,
): boolean {
  if (side === "new") {
    return change.type === "insert" || change.type === "normal";
  }
  return change.type === "delete";
}

/** Resolves path + line + source text for one quoteable gutter cell. */
export function diffQuoteAnchorFor(
  file: FileData,
  change: ChangeData,
  side: DiffQuoteSide,
  changeKey: string,
): DiffQuoteAnchor | null {
  if (!canQuoteDiffChange(change, side)) return null;
  const line = lineNumberForSide(change, side);
  if (line === null) return null;
  const path = side === "new" ? quoteNewPath(file) : file.oldPath;
  if (path.length === 0) return null;
  return {
    path,
    side,
    line,
    changeKey,
    content: stripDiffPrefix(change.content),
    changeType: change.type,
  };
}

/** Unified-diff body line so the agent sees add vs delete vs context. */
export function unifiedDiffQuoteLine(
  changeType: "insert" | "delete" | "normal",
  content: string,
): string {
  const mark =
    changeType === "insert" ? "+" : changeType === "delete" ? "-" : " ";
  return `${mark}${content}`;
}

function lineNumberForSide(
  change: ChangeData,
  side: DiffQuoteSide,
): number | null {
  if (change.type === "normal") {
    return side === "old" ? change.oldLineNumber : change.newLineNumber;
  }
  if (change.type === "delete") {
    return side === "old" ? change.lineNumber : null;
  }
  return side === "new" ? change.lineNumber : null;
}

function quoteNewPath(file: FileData): string {
  return file.type === "delete" ? file.oldPath : file.newPath;
}

/** Parser keeps git's leading marker; we strip then restore a canonical `+/-/ `. */
function stripDiffPrefix(content: string): string {
  if (
    content.startsWith("+") ||
    content.startsWith("-") ||
    content.startsWith(" ")
  ) {
    return content.slice(1);
  }
  return content;
}
