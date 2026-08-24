import type { ChatToolCall, ChatTurn } from "@ora/chat";
import { displayPath } from "../turn-diff-files";
import { normalizeDiffPath } from "../../../lib/workspace-path";
import { isLikelyFileArtifactPath, looksLikeGlobPattern } from "./parse";
import { collectToolOutputArtifacts } from "./tool-output-paths";

export interface SessionArtifactIndex {
  edited: string[];
  referenced: string[];
  directories?: string[];
  unknown?: string[];
}

export interface TurnArtifactCacheEntry {
  edited: string[];
  referenced: string[];
  directories?: string[];
  unknown?: string[];
  source?: ChatTurn;
  toolSources?: ChatToolCall[];
  cumulative?: SessionArtifactIndex;
  cumulativeParent?: TurnArtifactCacheEntry | null;
}

/** Stores one path after worktree-prefix stripping and slash normalization. */
function storedArtifactPath(path: string): string {
  return normalizeDiffPath(displayPath(path));
}

/** True when a tool path should enter the referenced set (files, not search roots). */
function isIndexableReferencedPath(path: string | null): path is string {
  return path !== null && isLikelyFileArtifactPath(path);
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

/** Adds a referenced file unless it is a directory, glob, or already edited. */
function addReferencedPath(
  rawPath: string | null,
  edited: Map<string, string>,
  referenced: Map<string, string>,
): void {
  if (!isIndexableReferencedPath(rawPath)) return;
  const path = storedArtifactPath(rawPath);
  const key = path.toLowerCase();
  if (edited.has(key)) return;
  referenced.set(key, path);
}

/** Keeps a concrete provider path unresolved when syntax cannot prove its kind. */
function addUnknownPath(
  rawPath: string | null,
  edited: Map<string, string>,
  referenced: Map<string, string>,
  unknown: Map<string, string>,
): void {
  if (
    rawPath === null ||
    rawPath.trim() === "" ||
    looksLikeGlobPattern(rawPath)
  ) {
    return;
  }
  const path = storedArtifactPath(rawPath).replace(/\/+$/, "");
  const key = path.toLowerCase();
  if (!edited.has(key) && !referenced.has(key)) unknown.set(key, path);
}

/** Collects edited and referenced paths for one turn without reading diff text. */
export function collectTurnArtifactPaths(turn: ChatTurn): {
  edited: string[];
  referenced: string[];
  directories?: string[];
  unknown?: string[];
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
  const directories = new Map<string, string>();
  const unknown = new Map<string, string>();
  for (const item of turn.items) {
    if (
      item.kind !== "toolCall" ||
      item.status === "failed" ||
      item.status === "cancelled"
    )
      continue;
    for (const location of item.locations) {
      if (/[\\/]$/.test(location.path)) {
        const path = storedArtifactPath(location.path).replace(/\/+$/, "");
        directories.set(path.toLowerCase(), path);
      } else {
        addReferencedPath(location.path, edited, referenced);
        if (!isIndexableReferencedPath(location.path)) {
          addUnknownPath(location.path, edited, referenced, unknown);
        }
      }
    }
    if (item.locations.length === 0) {
      const fallback = fallbackReadPath(item);
      addReferencedPath(fallback, edited, referenced);
      if (!isIndexableReferencedPath(fallback)) {
        addUnknownPath(fallback, edited, referenced, unknown);
      }
    }
    const outputArtifacts = collectToolOutputArtifacts(item);
    for (const outputPath of outputArtifacts.files) {
      const path = storedArtifactPath(outputPath);
      const key = path.toLowerCase();
      if (!edited.has(key)) referenced.set(key, path);
    }
    for (const outputDirectory of outputArtifacts.directories) {
      const path = storedArtifactPath(outputDirectory).replace(/\/+$/, "");
      directories.set(path.toLowerCase(), path);
    }
    for (const outputUnknown of outputArtifacts.unknown) {
      const path = storedArtifactPath(outputUnknown).replace(/\/+$/, "");
      unknown.set(path.toLowerCase(), path);
    }
  }

  return {
    edited: [...edited.values()],
    referenced: [...referenced.values()],
    ...(directories.size === 0
      ? {}
      : { directories: [...directories.values()] }),
    ...(unknown.size === 0 ? {} : { unknown: [...unknown.values()] }),
  };
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
  if (cache !== undefined) {
    const activeTurnIds = new Set(turns.map((turn) => turn.id));
    for (const turnId of cache.keys()) {
      if (!activeTurnIds.has(turnId)) cache.delete(turnId);
    }
  }
  const edited = new Map<string, string>();
  const referenced = new Map<string, string>();
  const directories = new Map<string, string>();
  const unknown = new Map<string, string>();
  const indices: SessionArtifactIndex[] = [];
  let previousEntry: TurnArtifactCacheEntry | null = null;
  let mapsHydrated = true;

  for (const turn of turns) {
    let entry = cache?.get(turn.id);
    const toolSources = turn.items.filter(
      (item): item is ChatToolCall => item.kind === "toolCall",
    );
    const toolsUnchanged =
      entry !== undefined &&
      entry.toolSources?.length === toolSources.length &&
      entry.toolSources.every((tool, index) => tool === toolSources[index]);
    if (entry !== undefined && entry.source !== turn && toolsUnchanged) {
      entry.source = turn;
    } else if (entry === undefined || entry.source !== turn) {
      const collected = collectTurnArtifactPaths(turn);
      entry = { source: turn, toolSources, ...collected };
      cache?.set(turn.id, entry);
    }

    if (
      entry.cumulative !== undefined &&
      entry.cumulativeParent === previousEntry
    ) {
      indices.push(entry.cumulative);
      previousEntry = entry;
      mapsHydrated = false;
      continue;
    }

    if (!mapsHydrated) {
      edited.clear();
      referenced.clear();
      directories.clear();
      unknown.clear();
      for (const path of previousEntry?.cumulative?.edited ?? []) {
        edited.set(path.toLowerCase(), path);
      }
      for (const path of previousEntry?.cumulative?.referenced ?? []) {
        referenced.set(path.toLowerCase(), path);
      }
      for (const path of previousEntry?.cumulative?.directories ?? []) {
        directories.set(path.toLowerCase(), path);
      }
      for (const path of previousEntry?.cumulative?.unknown ?? []) {
        unknown.set(path.toLowerCase(), path);
      }
      mapsHydrated = true;
    }

    for (const path of entry.edited) {
      const key = path.toLowerCase();
      edited.set(key, path);
      referenced.delete(key);
      directories.delete(key);
      unknown.delete(key);
    }
    for (const path of entry.referenced) {
      const key = path.toLowerCase();
      if (edited.has(key)) continue;
      referenced.set(key, path);
      directories.delete(key);
      unknown.delete(key);
    }
    for (const path of entry.directories ?? []) {
      const key = path.toLowerCase();
      if (edited.has(key)) continue;
      directories.set(key, path);
      referenced.delete(key);
      unknown.delete(key);
    }
    for (const path of entry.unknown ?? []) {
      const key = path.toLowerCase();
      if (!edited.has(key) && !referenced.has(key) && !directories.has(key)) {
        unknown.set(key, path);
      }
    }

    const currentReferenced = new Map(referenced);
    for (const key of edited.keys()) {
      currentReferenced.delete(key);
    }

    const cumulative: SessionArtifactIndex = {
      edited: [...edited.values()],
      referenced: [...currentReferenced.values()],
      ...(directories.size === 0
        ? {}
        : { directories: [...directories.values()] }),
      ...(unknown.size === 0 ? {} : { unknown: [...unknown.values()] }),
    };
    entry.cumulative = cumulative;
    entry.cumulativeParent = previousEntry;
    indices.push(cumulative);
    previousEntry = entry;
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
