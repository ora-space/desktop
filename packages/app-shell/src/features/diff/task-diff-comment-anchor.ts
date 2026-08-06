import { getChangeKey, type ChangeData, type FileData, type HunkData } from "react-diff-view";
import type { TaskDiffCommentAnchor, TaskDiffSide } from "@ora/contracts";

/** Builds the exact single-line anchor shape validated by the backend patch parser. */
export function createCommentAnchor(
  file: FileData,
  hunk: HunkData,
  change: ChangeData,
  side: TaskDiffSide,
  diffId: string,
): TaskDiffCommentAnchor {
  const lineNumber = lineNumberFor(change, side);
  if (lineNumber === null) {
    throw new Error(`change ${getChangeKey(change)} does not exist on the ${side} side`);
  }

  return {
    diffId,
    path: side === "old" ? file.oldPath : file.newPath,
    side,
    startLine: lineNumber,
    endLine: lineNumber,
    hunkHeader: hunk.content,
    // gitdiff-parser retains the CR from CRLF patches, while Rust `str::lines`
    // deliberately removes it before validating the source line.
    lineContent: change.content.replace(/\r$/, ""),
  };
}

/** Returns the old or new source line represented by one parsed change. */
function lineNumberFor(change: ChangeData, side: TaskDiffSide): number | null {
  if (change.type === "normal") {
    return side === "old" ? change.oldLineNumber : change.newLineNumber;
  }
  if (change.type === "delete") return side === "old" ? change.lineNumber : null;
  return side === "new" ? change.lineNumber : null;
}
