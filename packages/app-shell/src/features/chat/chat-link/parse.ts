import { isAbsoluteWorkspacePath } from "../../../lib/workspace-path";

/** Basenames that are files even without a dotted extension (Makefile, Dockerfile). */
const NAMED_WORKSPACE_FILES = new Set([
  "makefile",
  "dockerfile",
  "cargo.lock",
  "license",
  "notice",
  "copying",
  "changelog",
]);
const KNOWN_WORKSPACE_DIRECTORIES = new Set([
  ".git",
  ".github",
  ".idea",
  ".vscode",
  "node_modules",
]);

export interface ParsedPathCandidate {
  path: string;
  line: number | undefined;
  column: number | undefined;
}

export type ParsedChatHref =
  | { kind: "web"; href: string }
  | ({ kind: "file" } & ParsedPathCandidate)
  | { kind: "inert" };

const DANGEROUS_HREF_SCHEME = /^(javascript|data|vbscript|mailto):/i;
const HTTP_HREF_SCHEME = /^https?:/i;
const FILE_HREF_SCHEME = /^file:/i;
const HREF_SCHEME = /^([a-z][a-z0-9+.-]*):/i;

/** Decodes percent-escapes without throwing on malformed sequences. */
function decodeHref(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

/** Strips wrapping quotes/backticks/brackets and trailing prose punctuation. */
function sanitizeToken(raw: string): string {
  let value = decodeHref(raw.trim());
  value = stripTrailingPunctuation(value);
  if (
    (value.startsWith("`") && value.endsWith("`")) ||
    (value.startsWith('"') && value.endsWith('"')) ||
    (value.startsWith("'") && value.endsWith("'"))
  ) {
    value = value.slice(1, -1).trim();
  }
  const wrappers: ReadonlyArray<readonly [string, string]> = [
    ["(", ")"],
    ["[", "]"],
    ["{", "}"],
    ["<", ">"],
  ];
  const wrapper = wrappers.find(
    ([open, close]) => value.startsWith(open) && value.endsWith(close),
  );
  if (wrapper !== undefined) value = value.slice(1, -1).trim();
  return stripTrailingPunctuation(value);
}

/** Drops sentence punctuation and unmatched closing brackets after a path token. */
function stripTrailingPunctuation(value: string): string {
  let stripped = value.replace(/[,;.，；。！？、：]+$/, "");
  const pairs: ReadonlyArray<readonly [string, string]> = [
    ["(", ")"],
    ["[", "]"],
    ["{", "}"],
    ["<", ">"],
  ];
  let changed = true;
  while (changed) {
    changed = false;
    for (const [open, close] of pairs) {
      if (
        stripped.endsWith(close) &&
        countCharacter(stripped, close) > countCharacter(stripped, open)
      ) {
        stripped = stripped.slice(0, -1);
        changed = true;
      }
    }
  }
  return stripped;
}

/** Counts one bracket character when deciding whether trailing prose is unmatched. */
function countCharacter(value: string, target: string): number {
  return [...value].filter((character) => character === target).length;
}

/** Splits supported 1-based line/column suffixes from a path stem. */
function splitLineSuffix(value: string): ParsedPathCandidate {
  const linePhrase = value.match(
    /^(.*)\s+\(line\s+([1-9]\d*)(?:\s*,\s*col(?:umn)?\s+([1-9]\d*))?\)$/i,
  );
  if (linePhrase !== null) {
    return {
      path: linePhrase[1]!.trim(),
      line: Number(linePhrase[2]),
      column: linePhrase[3] === undefined ? undefined : Number(linePhrase[3]),
    };
  }

  const fragment = value.match(/^(.*?)#L([1-9]\d*)(?:C([1-9]\d*))?$/i);
  if (fragment !== null) {
    return {
      path: fragment[1]!,
      line: Number(fragment[2]),
      column: fragment[3] === undefined ? undefined : Number(fragment[3]),
    };
  }

  const query = value.match(
    /^(.*?)\?(?:line|ln)=([1-9]\d*)(?:&(?:column|col)=([1-9]\d*))?$/i,
  );
  if (query !== null) {
    return {
      path: query[1]!,
      line: Number(query[2]),
      column: query[3] === undefined ? undefined : Number(query[3]),
    };
  }

  const match = value.match(/^(.*?):([1-9]\d*)(?::([1-9]\d*))?$/);
  if (match === null) {
    return { path: value, line: undefined, column: undefined };
  }
  const stem = match[1]!;
  // A single letter before a colon is a Windows drive, not a line number.
  if (/^[A-Za-z]$/.test(stem)) {
    return { path: value, line: undefined, column: undefined };
  }
  return {
    path: stem,
    line: Number(match[2]),
    column: match[3] === undefined ? undefined : Number(match[3]),
  };
}

/** Strips a leading ./ so Markdown relative hrefs match workspace paths. */
function stripRelativePrefix(path: string): string {
  return path.replace(/^\.\//, "");
}

/** Parses one inline-code or href path token, including spaces and line numbers. */
export function parsePathCandidate(raw: string): ParsedPathCandidate {
  const parsed = splitLineSuffix(sanitizeToken(raw));
  return {
    path: stripRelativePrefix(parsed.path),
    line: parsed.line,
    column: parsed.column,
  };
}

/** True when a token is a glob/search pattern rather than a concrete file. */
export function looksLikeGlobPattern(path: string): boolean {
  return /[*?]/.test(path);
}

/**
 * True when the last path segment is a well-known filename or has an extension.
 * Used as the admission ticket for path-like inline code, not as proof the file exists.
 */
function isNamedWorkspaceFile(path: string): boolean {
  const filename = path.split(/[\\/]/).at(-1)?.toLowerCase() ?? "";
  if (filename === "") return false;
  if (NAMED_WORKSPACE_FILES.has(filename)) return true;
  if (!path.includes("/") && !path.includes("\\") && /\s/.test(filename)) {
    return false;
  }
  const extension = filename.includes(".")
    ? (filename.split(".").at(-1) ?? "")
    : "";
  return extension.length > 0 && !/\s/.test(extension);
}

/**
 * Path-like is only an admission ticket for inline code. Linking still requires
 * a session-index hit. Commands and type names stay plain text.
 */
export function isPathLikeToken(raw: string): boolean {
  const { path } = parsePathCandidate(raw);
  if (looksLikeGlobPattern(path)) return false;
  if (path.includes("/") || path.includes("\\")) return true;
  if (isAbsoluteWorkspacePath(path)) return true;
  return isNamedWorkspaceFile(path);
}

/**
 * True when a tool location or dump line looks like a file, not a search root
 * or glob pattern. Directories without a trailing slash still fail this check
 * because they have no extension.
 */
export function isLikelyFileArtifactPath(path: string): boolean {
  const trimmed = path.trim();
  if (
    trimmed === "" ||
    looksLikeGlobPattern(trimmed) ||
    /[\\/]$/.test(trimmed)
  ) {
    return false;
  }
  return isNamedWorkspaceFile(trimmed);
}

/** True when tool output explicitly looks like a directory rather than a file. */
export function isLikelyDirectoryArtifactPath(path: string): boolean {
  const trimmed = path.trim();
  if (trimmed === "" || looksLikeGlobPattern(trimmed)) return false;
  if (/[\\/]$/.test(trimmed)) return true;
  const parsed = parsePathCandidate(trimmed).path;
  const filename = parsed.split(/[\\/]/).at(-1)?.toLowerCase() ?? "";
  if (KNOWN_WORKSPACE_DIRECTORIES.has(filename)) return true;
  return (
    (parsed.includes("/") || parsed.includes("\\")) &&
    !isLikelyFileArtifactPath(parsed)
  );
}

/**
 * Classifies a Markdown href without probing the workspace.
 * Web hrefs keep their original encoding; file paths decode percent-escapes once.
 */
export function parseChatHref(href: string): ParsedChatHref {
  const trimmed = href.trim();
  if (HTTP_HREF_SCHEME.test(trimmed)) {
    return { kind: "web", href: trimmed };
  }
  if (DANGEROUS_HREF_SCHEME.test(trimmed)) {
    return { kind: "inert" };
  }
  const scheme = trimmed.match(HREF_SCHEME)?.[1];
  // Single-letter schemes are Windows drives (`C:`), not URL protocols.
  if (
    scheme !== undefined &&
    !FILE_HREF_SCHEME.test(trimmed) &&
    !/^[A-Za-z]$/.test(scheme)
  ) {
    return { kind: "inert" };
  }
  if (FILE_HREF_SCHEME.test(trimmed)) {
    let rest = trimmed.replace(FILE_HREF_SCHEME, "").replace(/^\/\//, "");
    if (/^\/[A-Za-z]:/.test(rest)) rest = rest.slice(1);
    const parsed = parsePathCandidate(rest);
    return { kind: "file", ...parsed };
  }
  if (
    trimmed.startsWith("/") ||
    /^\.\.?\//.test(trimmed) ||
    /^[A-Za-z]:[\\/]/.test(trimmed) ||
    isPathLikeToken(trimmed) ||
    trimmed.includes("/") ||
    trimmed.includes("\\")
  ) {
    const parsed = parsePathCandidate(trimmed);
    return { kind: "file", ...parsed };
  }
  return { kind: "inert" };
}
