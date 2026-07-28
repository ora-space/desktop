import { diffLines } from "diff";
import type { ChatTurn } from "@ora/chat";

export interface TurnDiffFile {
  path: string;
  oldText: string;
  newText: string;
  additions: number;
  deletions: number;
}

/** Merges repeated edits to a path so the summary represents its complete turn-level change. */
export function collectTurnDiffFiles(turn: ChatTurn): TurnDiffFile[] {
  const files = new Map<string, { path: string; oldText: string; newText: string }>();

  for (const item of turn.items) {
    if (item.kind !== "toolCall" || item.status !== "completed") continue;
    for (const content of item.content) {
      if (content.type !== "diff") continue;
      const existing = files.get(content.path);
      files.set(content.path, {
        path: content.path,
        oldText: existing?.oldText ?? content.oldText ?? "",
        newText: content.newText,
      });
    }
  }

  return [...files.values()].map((file) => ({
    ...file,
    ...countTextChanges(file.oldText, file.newText),
  }));
}

/** Counts changed lines using the same line-diff semantics as the rendered viewer. */
function countTextChanges(oldText: string, newText: string): {
  additions: number;
  deletions: number;
} {
  let additions = 0;
  let deletions = 0;
  for (const part of diffLines(oldText, newText)) {
    const lineCount = part.value.endsWith("\n") ? part.count ?? 0 : (part.count ?? 1);
    if (part.added) additions += lineCount;
    if (part.removed) deletions += lineCount;
  }
  return { additions, deletions };
}
