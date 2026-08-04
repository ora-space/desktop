import { diffLines } from "diff";
import type { ChatToolCall, ChatTurn } from "@ora/chat";

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
    let receivedProtocolDiff = false;
    for (const content of item.content) {
      if (content.type !== "diff") continue;
      receivedProtocolDiff = true;
      const existing = files.get(content.path);
      files.set(content.path, {
        path: content.path,
        oldText: existing?.oldText ?? content.oldText ?? "",
        newText: content.newText,
      });
    }

    // Some ACP adapters classify write tools as edits but only preserve their raw input.
    // Falling back to the full written content keeps provider choice from hiding new files.
    if (!receivedProtocolDiff) {
      const fallback = fullContentWriteDiff(item);
      if (fallback !== null) {
        const existing = files.get(fallback.path);
        files.set(fallback.path, {
          ...fallback,
          oldText: existing?.oldText ?? fallback.oldText,
        });
      }
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

/** Converts a provider's full-content write input into a new-file diff when ACP omitted one. */
function fullContentWriteDiff(
  tool: ChatToolCall,
): { path: string; oldText: string; newText: string } | null {
  if (tool.toolKind !== "edit" || !isRecord(tool.rawInput)) return null;

  const newText = stringField(tool.rawInput, ["content", "newText", "new_text"]);
  if (newText === null) return null;

  const rawPath = tool.locations.at(-1)?.path
    ?? stringField(tool.rawInput, ["filePath", "file_path", "path"]);
  if (rawPath === null) return null;

  return {
    path: displayPath(rawPath),
    oldText: stringField(tool.rawInput, ["oldText", "old_text"]) ?? "",
    newText,
  };
}

/** Narrows unknown provider payloads before reading their fields. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Returns the first string value used by one of the supported provider field names. */
function stringField(
  value: Record<string, unknown>,
  fieldNames: string[],
): string | null {
  for (const fieldName of fieldNames) {
    const field = value[fieldName];
    if (typeof field === "string") return field;
  }
  return null;
}

/** Removes the private managed-worktree prefix from absolute provider paths. */
function displayPath(path: string): string {
  const managedWorktreePath = path.match(
    /(?:^|[\\/])worktrees[\\/][^\\/]+[\\/](.+)$/,
  )?.[1];
  return managedWorktreePath?.replaceAll("\\", "/") ?? path;
}
