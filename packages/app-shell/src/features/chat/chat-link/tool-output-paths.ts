import type { ChatToolCall } from "@ora/chat";
import {
  isLikelyDirectoryArtifactPath,
  isLikelyFileArtifactPath,
  isPathLikeToken,
  looksLikeGlobPattern,
  parseChatHref,
  parsePathCandidate,
} from "./parse";
import { isAbsoluteWorkspacePath } from "../../../lib/workspace-path";
import {
  powerShellEntryReader,
  powerShellModeEntry,
  stripAnsi,
} from "./tool-output-table";

const OUTPUT_PATH_KEYS = [
  "output",
  "entries",
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

export interface ToolOutputArtifacts {
  files: string[];
  directories: string[];
  unknown: string[];
}

/** Strips a leading markdown/plain list marker so dump lines can be path-checked. */
export function stripListMarker(line: string): string {
  let current = line.trim();
  current = current.replace(/^[│\s]*[├└]──\s*/, "");
  current = current.replace(/^[-*+]\s+/, "");
  current = current.replace(/^\d+[.)]\s+/, "");
  current = current.replace(/^\[[ xX]\]\s+/, "");
  return current.trim();
}

/** Collects concrete file paths from one newline-oriented glob/search dump. */
export function extractArtifactPathsFromText(text: string): string[] {
  const paths: string[] = [];
  const directoryKeys = new Set(
    extractArtifactDirectoriesFromText(text).map(normalizedPathKey),
  );
  const lines = text.split(/\r?\n/).map(stripAnsi);
  const typedEntry = powerShellEntryReader(lines);
  for (const rawLine of lines) {
    const modeEntry = typedEntry(rawLine);
    if (modeEntry !== null) {
      if (modeEntry.kind === "file") paths.push(modeEntry.path);
      continue;
    }
    const token = pathTokenFromOutputLine(rawLine);
    if (token === null) continue;
    const parsed = parsePathCandidate(token).path;
    if (directoryKeys.has(normalizedPathKey(parsed))) continue;
    pushIndexablePath(token, paths);
  }
  return paths;
}

/** Collects explicit directory paths from newline-oriented tool output. */
export function extractArtifactDirectoriesFromText(text: string): string[] {
  const directories: string[] = [];
  const lines = text.split(/\r?\n/).map(stripAnsi);
  const typedEntry = powerShellEntryReader(lines);
  for (const rawLine of lines) {
    const modeEntry = typedEntry(rawLine);
    if (modeEntry?.kind === "directory") {
      directories.push(modeEntry.path);
      continue;
    }
    const token = pathTokenFromOutputLine(rawLine);
    if (token !== null) {
      const path = directoryPathFromToken(token);
      if (path !== null) directories.push(path);
      continue;
    }
  }
  return directories;
}

/** Extracts a path token from path-only or ripgrep-style output lines. */
export function pathTokenFromOutputLine(rawLine: string): string | null {
  const modeEntry = powerShellModeEntry(rawLine);
  if (modeEntry !== null) return modeEntry.path;
  const line = stripListMarker(rawLine);
  if (line === "") return null;

  const location =
    line.match(/^(.*?):([1-9]\d*):([1-9]\d*):/) ??
    line.match(/^(.*?):([1-9]\d*):/);
  if (location !== null) {
    const token = `${location[1]}:${location[2]}${location[3] === undefined ? "" : `:${location[3]}`}`;
    if (indexablePathFromToken(token) !== null) return token;
  }
  return indexablePathFromToken(line) === null &&
    directoryPathFromToken(line) === null
    ? null
    : line;
}

/** True for one plain entry name in a newline list, excluding headings and commands. */
function isBareArtifactName(value: string): boolean {
  return (
    value !== "" &&
    !/\s/.test(value) &&
    !looksLikeGlobPattern(value) &&
    /^[\w.@+()-]+$/u.test(value)
  );
}

/**
 * True for a relative listing entry such as `.claude/commands`. A recursive
 * listing prints one child per line with no kind column, and those lines are
 * otherwise dropped: a directory has no extension, so the file heuristics
 * reject it and only the top level (bare names) survives. Ripgrep-style
 * `path:line:column:text` output is excluded, since it is a search hit rather
 * than a listing entry.
 */
function isRelativeListingPath(value: string): boolean {
  if (value === "" || /\s/.test(value) || value.includes(":")) return false;
  if (looksLikeGlobPattern(value)) return false;
  const segments = value.split(/[\\/]/).filter((segment) => segment !== "");
  return segments.length > 1 && segments.every(isBareArtifactName);
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

/** Collects directory candidates only from visible text output. */
export function collectToolOutputDirectories(tool: ChatToolCall): string[] {
  const directories: string[] = [];
  for (const content of tool.content) {
    if (content.type !== "content" || content.content.type !== "text") continue;
    directories.push(
      ...extractArtifactDirectoriesFromText(content.content.text),
    );
  }
  return uniquePaths(directories);
}

/** Parses one tool output once and keeps ambiguous listing entries unresolved. */
export function collectToolOutputArtifacts(
  tool: ChatToolCall,
): ToolOutputArtifacts {
  const files: string[] = [];
  const directories: string[] = [];
  const unknown: string[] = [];
  const typedFiles: string[] = [];
  const typedDirectories: string[] = [];
  const listing = isDirectoryListingTool(tool);
  const listingRoot = listing ? directoryListingRoot(tool) : null;
  /** Kind the visible listing established for one entry, by both of its forms. */
  const listedKind = new Map<string, "file" | "directory" | "unknown">();
  const listed = (
    path: string,
    kind: "file" | "directory" | "unknown",
  ): string => {
    const qualified = qualifyListingPath(path, listingRoot);
    // The verdict is recorded under both forms so a bare `rawOutput` guess for
    // the same entry can be recognized, even though only the qualified form is
    // indexed.
    listedKind.set(normalizedPathKey(path), kind);
    listedKind.set(normalizedPathKey(qualified), kind);
    return qualified;
  };
  for (const content of tool.content) {
    if (content.type !== "content" || content.content.type !== "text") continue;
    const parsed = parseToolOutputText(content.content.text, listing);
    files.push(...parsed.files.map((path) => listed(path, "file")));
    directories.push(
      ...parsed.directories.map((path) => listed(path, "directory")),
    );
    unknown.push(...parsed.unknown.map((path) => listed(path, "unknown")));
  }
  if (tool.rawOutput !== undefined) {
    collectRawOutputArtifacts(
      tool.rawOutput,
      files,
      directories,
      typedFiles,
      typedDirectories,
      0,
    );
  }
  files.push(...typedFiles);
  directories.push(...typedDirectories);
  const typedFileKeys = new Set(typedFiles.map(normalizedPathKey));
  const typedDirectoryKeys = new Set(typedDirectories.map(normalizedPathKey));
  const explicitKeys = new Set([...typedFileKeys, ...typedDirectoryKeys]);
  const uniqueUnknown = uniquePaths(unknown).filter(
    (path) => !explicitKeys.has(normalizedPathKey(path)),
  );
  /**
   * `rawOutput` repeats the same listing text, and its file heuristics guess:
   * `.claude` reads as a dotfile there. The visible listing is the better
   * witness, so its verdict (including "unresolved") wins over that guess.
   */
  const contradictsListing = (
    path: string,
    kind: "file" | "directory",
  ): boolean => {
    const key = normalizedPathKey(path);
    // Structured provider kinds are evidence, not a guess, and outrank the
    // visible listing.
    if (explicitKeys.has(key)) return false;
    const verdict = listedKind.get(key);
    return verdict !== undefined && verdict !== kind;
  };
  return {
    files: uniquePaths(files).filter(
      (path) =>
        !typedDirectoryKeys.has(normalizedPathKey(path)) &&
        !contradictsListing(path, "file"),
    ),
    directories: uniquePaths(directories).filter(
      (path) =>
        !typedFileKeys.has(normalizedPathKey(path)) &&
        !contradictsListing(path, "directory"),
    ),
    unknown: uniqueUnknown,
  };
}

/** Finds the directory whose child names a listing command returned. */
function directoryListingRoot(tool: ChatToolCall): string | null {
  const input = isRecord(tool.rawInput) ? tool.rawInput : null;
  const explicitPath =
    input === null
      ? null
      : ["filePath", "file_path", "path", "AbsolutePath", "absolute_path"]
          .map((key) => input[key])
          .find((value): value is string => typeof value === "string");
  const command =
    input === null
      ? null
      : Object.values(input).find(
          (value): value is string =>
            typeof value === "string" && /\bGet-ChildItem\b/i.test(value),
        );
  const commandPath =
    command === null || command === undefined
      ? null
      : powerShellListingPath(command);
  const base =
    tool.locations.at(-1)?.path ??
    (typeof input?.cwd === "string" ? input.cwd : null);
  const requested = explicitPath ?? commandPath;
  if (requested === null || requested === ".") return base;
  if (isAbsoluteWorkspacePath(requested) || base === null) return requested;
  const root = base.replaceAll("\\", "/").replace(/\/+$/, "");
  const relative = requested.replaceAll("\\", "/").replace(/^\.\//, "");
  if (root.toLowerCase().endsWith(`/${relative.toLowerCase()}`)) return root;
  return `${root}/${relative}`;
}

/** Extracts an explicit or positional path argument from Get-ChildItem. */
function powerShellListingPath(command: string): string | null {
  const flagged = command.match(
    /-(?:LiteralPath|Path)\s+(?:"([^"]+)"|'([^']+)'|([^\s|]+))/i,
  );
  if (flagged !== null) return flagged[1] ?? flagged[2] ?? flagged[3] ?? null;
  const positional = command.match(
    /\bGet-ChildItem\b\s+(?:"([^"]+)"|'([^']+)'|([^\s|]+))/i,
  );
  const value = positional?.[1] ?? positional?.[2] ?? positional?.[3];
  return value === undefined || value.startsWith("-") ? null : value;
}

/** Prefixes relative listing children with the directory that owns them. */
function qualifyListingPath(path: string, root: string | null): string {
  if (root === null || isAbsoluteWorkspacePath(path)) return path;
  const normalizedRoot = root.replaceAll("\\", "/").replace(/\/+$/, "");
  const child = path.replaceAll("\\", "/").replace(/^\.\//, "");
  return normalizedRoot === "" ? child : `${normalizedRoot}/${child}`;
}

/** Traverses provider output once while retaining explicit type evidence separately. */
function collectRawOutputArtifacts(
  value: unknown,
  files: string[],
  directories: string[],
  typedFiles: string[],
  typedDirectories: string[],
  depth: number,
): void {
  if (depth > 4) return;
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (trimmed.startsWith("[") || trimmed.startsWith("{")) {
      try {
        collectRawOutputArtifacts(
          JSON.parse(trimmed) as unknown,
          files,
          directories,
          typedFiles,
          typedDirectories,
          depth + 1,
        );
        return;
      } catch {
        // Provider strings are often ordinary text rather than embedded JSON.
      }
    }
    if (trimmed.includes("\n")) {
      files.push(...extractArtifactPathsFromText(trimmed));
      directories.push(...extractArtifactDirectoriesFromText(trimmed));
    } else if (/[\\/]$/.test(trimmed)) {
      directories.push(trimmed.replace(/[\\/]+$/, ""));
    } else {
      pushIndexablePath(trimmed, files);
    }
    return;
  }
  if (Array.isArray(value)) {
    for (const item of value) {
      collectRawOutputArtifacts(
        item,
        files,
        directories,
        typedFiles,
        typedDirectories,
        depth + 1,
      );
    }
    return;
  }
  if (!isRecord(value)) return;
  const kind = value.kind ?? value.type;
  const path = value.path ?? value.filePath ?? value.file_path;
  if (typeof path === "string" && (kind === "file" || kind === "directory")) {
    (kind === "file" ? typedFiles : typedDirectories).push(
      path.replace(/[\\/]+$/, ""),
    );
  }
  for (const [key, field] of Object.entries(value)) {
    if (
      key === "kind" ||
      key === "type" ||
      (field === path && (kind === "file" || kind === "directory"))
    ) {
      continue;
    }
    if (
      !OUTPUT_ITEM_PATH_KEYS.includes(
        key as (typeof OUTPUT_ITEM_PATH_KEYS)[number],
      ) &&
      !OUTPUT_PATH_KEYS.includes(key as (typeof OUTPUT_PATH_KEYS)[number])
    ) {
      continue;
    }
    collectRawOutputArtifacts(
      field,
      files,
      directories,
      typedFiles,
      typedDirectories,
      depth + 1,
    );
  }
}

/** Extracts all artifact kinds from one text payload in a single line pass. */
function parseToolOutputText(
  text: string,
  listing: boolean,
): ToolOutputArtifacts {
  const files: string[] = [];
  const directories: string[] = [];
  const unknown: string[] = [];
  const lines = text.split(/\r?\n/).map(stripAnsi);
  // Either table shape carries explicit kind evidence for the rows below it.
  const typedEntry = powerShellEntryReader(lines);
  const hasModeEntries = lines.some((line) => typedEntry(line) !== null);
  for (const rawLine of lines) {
    const modeEntry = typedEntry(rawLine);
    if (modeEntry !== null) {
      (modeEntry.kind === "file" ? files : directories).push(modeEntry.path);
      continue;
    }
    const line = stripListMarker(rawLine);
    if (line === "" || looksLikeGlobPattern(line)) continue;
    if (/[\\/]$/.test(line)) {
      directories.push(parsePathCandidate(line).path.replace(/[\\/]+$/, ""));
      continue;
    }
    if (
      listing &&
      !hasModeEntries &&
      (isBareArtifactName(line) ||
        isAbsoluteWorkspacePath(line) ||
        isRelativeListingPath(line))
    ) {
      unknown.push(parsePathCandidate(line).path);
      continue;
    }
    if (listing && !hasModeEntries) {
      const columns = line
        .split(/\s{2,}/)
        .map((column) => column.trim())
        .filter((column) => column !== "");
      if (columns.length > 1) {
        unknown.push(...columns);
        continue;
      }
    }
    const token = pathTokenFromOutputLine(line);
    if (token === null) continue;
    const file = indexablePathFromToken(token);
    if (file !== null) files.push(file);
  }
  return { files, directories, unknown };
}

/** True when tool metadata identifies a directory listing operation. */
function isDirectoryListingTool(tool: ChatToolCall): boolean {
  const command = isRecord(tool.rawInput)
    ? Object.values(tool.rawInput)
        .filter((value): value is string => typeof value === "string")
        .join(" ")
    : "";
  const description = `${tool.title} ${command}`;
  const rawOutput = JSON.stringify(tool.rawOutput ?? "");
  return (
    /\b(?:Get-ChildItem|gci|dir|ls)\b|list.*director|read.*director/i.test(
      description,
    ) || /<type>directory<\/type>|"type"\s*:\s*"directory"/i.test(rawOutput)
  );
}

/** True when a fenced block is a path list rather than source code. */
export function isPlainPathList(code: string, language: string): boolean {
  if (language !== "text" && language !== "plaintext") return false;
  const rawLines = code.split(/\r?\n/);
  const tree = rawLines.some((line) => /^[│\s]*[├└]──/.test(line.trim()));
  const lines = rawLines
    .map((line) => stripListMarker(line))
    .filter((line) => line !== "");
  return (
    lines.length > 0 &&
    lines.every((line) => {
      if (tree && isBareArtifactName(line)) return true;
      if (!isPathLikeToken(line)) return false;
      const path = parsePathCandidate(line).path;
      return (
        isLikelyFileArtifactPath(path) || isLikelyDirectoryArtifactPath(path)
      );
    })
  );
}

/** Narrows unknown provider payloads before reading their fields. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Adds one path-like token when it looks like a concrete file. */
function pushIndexablePath(raw: string, paths: string[]): void {
  const path = indexablePathFromToken(raw);
  if (path !== null) paths.push(path);
}

/** Returns the normalized artifact path represented by one candidate token. */
function indexablePathFromToken(raw: string): string | null {
  const href = parseChatHref(raw);
  if (href.kind === "web" || href.kind === "inert") return null;
  const token = href.path;
  if (!isPathLikeToken(token)) return null;
  const { path } = parsePathCandidate(token);
  return isLikelyFileArtifactPath(path) ? path : null;
}

/** Returns the normalized directory path represented by one candidate token. */
function directoryPathFromToken(raw: string): string | null {
  const href = parseChatHref(raw);
  if (href.kind === "web" || href.kind === "inert") return null;
  const token = href.path;
  const { path } = parsePathCandidate(token);
  return /[\\/]$/.test(path) ? path.replace(/[\\/]+$/, "") : null;
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

/** Normalizes one extracted artifact for cross-style file/directory exclusion. */
function normalizedPathKey(path: string): string {
  return path.replaceAll("\\", "/").replace(/\/+$/, "").toLowerCase();
}
