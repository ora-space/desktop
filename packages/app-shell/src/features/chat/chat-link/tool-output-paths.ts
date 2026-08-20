import type { ChatToolCall } from "@ora/chat";
import {
  isLikelyFileArtifactPath,
  isPathLikeToken,
  parseChatHref,
  parsePathCandidate,
} from "./parse";

const OUTPUT_PATH_KEYS = [
  "filenames",
  "files",
  "paths",
  "matches",
  "results",
] as const;
const OUTPUT_ITEM_PATH_KEYS = [
  "path",
  "filePath",
  "file_path",
  "filename",
  "file",
  "uri",
] as const;

/** Strips a leading markdown/plain list marker so dump lines can be path-checked. */
export function stripListMarker(line: string): string {
  let current = line.trim();
  current = current.replace(/^[-*+]\s+/, "");
  current = current.replace(/^\d+[.)]\s+/, "");
  current = current.replace(/^\[[ xX]\]\s+/, "");
  return current.trim();
}

/** Collects concrete file paths from one newline-oriented glob/search dump. */
export function extractArtifactPathsFromText(text: string): string[] {
  const paths: string[] = [];
  for (const rawLine of text.split(/\r?\n/)) {
    const line = stripListMarker(rawLine);
    if (line === "") continue;
    pushIndexablePath(line, paths);
  }
  return paths;
}

/**
 * Collects file paths from search/read tool dumps. Glob results often arrive as
 * text or a filename array instead of per-file ACP locations.
 */
export function collectToolOutputPaths(tool: ChatToolCall): string[] {
  const paths: string[] = [];
  for (const content of tool.content) {
    if (content.type !== "content" || content.content.type !== "text") continue;
    paths.push(...extractArtifactPathsFromText(content.content.text));
  }
  if (tool.rawOutput !== undefined) {
    collectPathsFromUnknown(tool.rawOutput, paths, 0);
  }
  return uniquePaths(paths);
}

/** True when a fenced block is a path list rather than source code. */
export function isPlainPathList(code: string, language: string): boolean {
  if (language !== "text" && language !== "plaintext") return false;
  const lines = code
    .split(/\r?\n/)
    .map((line) => stripListMarker(line))
    .filter((line) => line !== "");
  return (
    lines.length > 0 &&
    lines.every((line) => {
      if (!isPathLikeToken(line)) return false;
      return isLikelyFileArtifactPath(parsePathCandidate(line).path);
    })
  );
}

/** Narrows unknown provider payloads before reading their fields. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Adds one path-like token when it looks like a concrete file. */
function pushIndexablePath(raw: string, paths: string[]): void {
  const href = parseChatHref(raw);
  const token = href.kind === "file" ? href.path : raw;
  if (!isPathLikeToken(token)) return;
  const { path } = parsePathCandidate(token);
  if (!isLikelyFileArtifactPath(path)) return;
  paths.push(path);
}

/** Walks a small JSON dump without treating glob `path` inputs as files. */
function collectPathsFromUnknown(
  value: unknown,
  paths: string[],
  depth: number,
): void {
  if (depth > 4) return;
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (trimmed.startsWith("[") || trimmed.startsWith("{")) {
      try {
        collectPathsFromUnknown(
          JSON.parse(trimmed) as unknown,
          paths,
          depth + 1,
        );
        return;
      } catch {
        // Fall through and treat the string as a path dump.
      }
    }
    if (trimmed.includes("\n")) {
      paths.push(...extractArtifactPathsFromText(trimmed));
      return;
    }
    pushIndexablePath(trimmed, paths);
    return;
  }
  if (Array.isArray(value)) {
    for (const item of value) {
      collectPathsFromUnknown(item, paths, depth + 1);
    }
    return;
  }
  if (!isRecord(value)) return;
  for (const key of OUTPUT_ITEM_PATH_KEYS) {
    const field = value[key];
    if (typeof field === "string") {
      pushIndexablePath(field, paths);
    } else if (Array.isArray(field)) {
      for (const item of field) {
        if (typeof item === "string") pushIndexablePath(item, paths);
      }
    }
  }
  for (const key of OUTPUT_PATH_KEYS) {
    if (key in value) {
      collectPathsFromUnknown(value[key], paths, depth + 1);
    }
  }
}

/** Dedupes extracted paths while keeping first-seen provider casing. */
function uniquePaths(paths: string[]): string[] {
  const seen = new Set<string>();
  const unique: string[] = [];
  for (const path of paths) {
    const key = path.replaceAll("\\", "/").toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    unique.push(path);
  }
  return unique;
}
