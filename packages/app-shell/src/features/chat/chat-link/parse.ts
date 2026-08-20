import { isAbsoluteWorkspacePath } from "../../../lib/workspace-path";

/** Basenames that are files even without a dotted extension (Makefile, Dockerfile). */
const NAMED_WORKSPACE_FILES = new Set(["makefile", "dockerfile", "cargo.lock"]);

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

/** Strips wrapping quotes/backticks and trailing prose punctuation that is not an extension. */
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
  return stripTrailingPunctuation(value);
}

/** Drops commas/semicolons and a leftover period after a completed path token. */
function stripTrailingPunctuation(value: string): string {
  return value.replace(/[,;]+$/, "").replace(/\.$/, "");
}

/** Splits an optional 1-based :line / :line:col or ` (line N)` suffix from a path stem. */
function splitLineSuffix(value: string): ParsedPathCandidate {
  const linePhrase = value.match(/^(.*)\s+\(line\s+(\d+)\)$/i);
  if (linePhrase !== null) {
    return {
      path: linePhrase[1]!.trim(),
      line: Number(linePhrase[2]),
      column: undefined,
    };
  }

  const match = value.match(/^(.*?):(\d+)(?::(\d+))?$/);
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
