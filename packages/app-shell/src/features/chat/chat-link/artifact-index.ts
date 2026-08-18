import type { ChatToolCall, ChatTurn } from "@ora/chat";
import { displayPath } from "../turn-diff-files";
import { normalizeDiffPath } from "../../../lib/workspace-path";

export interface SessionArtifactIndex {
  edited: string[];
  referenced: string[];
}

export interface TurnArtifactCacheEntry {
  fingerprint: string;
  edited: string[];
  referenced: string[];
}

/** Stores one path after worktree-prefix stripping and slash normalization. */
function storedArtifactPath(path: string): string {
  return normalizeDiffPath(displayPath(path));
}

/** Directory locations are not Files-preview targets in v1. */
function isDirectoryPath(path: string): boolean {
  return /[\\/]$/.test(path.trim());
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

/**
 * Path-only fallback matching collectTurnDiffFiles: edit tools that wrote content
 * but never received an ACP diff still count as edited artifacts.
 */
function fallbackEditPath(tool: ChatToolCall): string | null {
  if (tool.toolKind !== "edit" || !isRecord(tool.rawInput)) return null;
  const newText = stringField(tool.rawInput, [
    "content",
    "newText",
    "new_text",
  ]);
  if (newText === null) return null;
  return (
    tool.locations.at(-1)?.path ??
    stringField(tool.rawInput, ["filePath", "file_path", "path"])
  );
}

/** Extracts candidate read/reference paths from tool inputs when ACP emitted no locations. */
function fallbackReadPath(tool: ChatToolCall): string | null {
  if (tool.toolKind === "edit" || !isRecord(tool.rawInput)) return null;
  return (
    tool.locations.at(-1)?.path ??
    stringField(tool.rawInput, [
      "filePath",
      "file_path",
      "path",
      "AbsolutePath",
      "absolute_path",
      "targetFile",
      "target_file",
      "uri",
    ])
  );
}

/** Collects edited and referenced paths for one turn without reading diff text. */
export function collectTurnArtifactPaths(turn: ChatTurn): {
  edited: string[];
  referenced: string[];
} {
  const edited = new Map<string, string>();

  for (const item of turn.items) {
    if (
      item.kind !== "toolCall" ||
      item.status === "failed" ||
      item.status === "cancelled"
    )
      continue;
    let receivedProtocolDiff = false;
    for (const content of item.content) {
      if (content.type !== "diff") continue;
      receivedProtocolDiff = true;
      const path = storedArtifactPath(content.path);
      edited.set(path.toLowerCase(), path);
    }
    if (!receivedProtocolDiff) {
      const fallback = fallbackEditPath(item);
      if (fallback !== null) {
        const path = storedArtifactPath(fallback);
        edited.set(path.toLowerCase(), path);
      }
    }
  }

  const referenced = new Map<string, string>();
  for (const item of turn.items) {
    if (
      item.kind !== "toolCall" ||
      item.status === "failed" ||
      item.status === "cancelled"
    )
      continue;
    for (const location of item.locations) {
      if (isDirectoryPath(location.path)) continue;
      const path = storedArtifactPath(location.path);
      const key = path.toLowerCase();
      if (edited.has(key)) continue;
      referenced.set(key, path);
    }
    if (item.locations.length === 0) {
      const fallback = fallbackReadPath(item);
      if (fallback !== null && !isDirectoryPath(fallback)) {
        const path = storedArtifactPath(fallback);
        const key = path.toLowerCase();
        if (!edited.has(key)) {
          referenced.set(key, path);
        }
      }
    }
  }

  return {
    edited: [...edited.values()],
    referenced: [...referenced.values()],
  };
}

/** Fingerprint of the path-bearing tool fields so completed turns can be reused while streaming. */
function turnArtifactFingerprint(turn: ChatTurn): string {
  return turn.items
    .filter((item): item is ChatToolCall => item.kind === "toolCall")
    .map((item) => {
      const diffs = item.content
        .filter((content) => content.type === "diff")
        .map((diff) => diff.path)
        .join(",");
      const locations = item.locations.map((loc) => loc.path).join(",");
      const editFallback = fallbackEditPath(item) ?? "";
      const readFallback = fallbackReadPath(item) ?? "";
      return `${item.id}:${item.status ?? ""}:${diffs}:${locations}:${editFallback}:${readFallback}`;
    })
    .join(";");
}

/**
 * Builds cumulative artifact indices for each turn in order.
 * Turn i only includes files edited or referenced up to turn i, so prior turns
 * where a file was only read maintain read-only links to Files.
 */
export function collectCumulativeArtifactIndices(
  turns: ChatTurn[],
  cache?: Map<string, TurnArtifactCacheEntry>,
): SessionArtifactIndex[] {
  const edited = new Map<string, string>();
  const referenced = new Map<string, string>();
  const indices: SessionArtifactIndex[] = [];

  for (const turn of turns) {
    const fingerprint = turnArtifactFingerprint(turn);
    let entry = cache?.get(turn.id);
    if (entry === undefined || entry.fingerprint !== fingerprint) {
      const collected = collectTurnArtifactPaths(turn);
      entry = { fingerprint, ...collected };
      cache?.set(turn.id, entry);
    }

    for (const path of entry.edited) {
      edited.set(path.toLowerCase(), path);
    }
    for (const path of entry.referenced) {
      referenced.set(path.toLowerCase(), path);
    }

    const currentReferenced = new Map(referenced);
    for (const key of edited.keys()) {
      currentReferenced.delete(key);
    }

    indices.push({
      edited: [...edited.values()],
      referenced: [...currentReferenced.values()],
    });
  }

  return indices;
}

/**
 * Builds the session-wide edited/referenced sets. Pass a cache Map so only the
 * live streaming turn is recomputed while earlier turns stay memoized.
 */
export function collectSessionArtifactIndex(
  turns: ChatTurn[],
  cache?: Map<string, TurnArtifactCacheEntry>,
): SessionArtifactIndex {
  const cumulative = collectCumulativeArtifactIndices(turns, cache);
  return cumulative.at(-1) ?? { edited: [], referenced: [] };
}
