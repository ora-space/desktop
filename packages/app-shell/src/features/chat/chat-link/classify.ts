import { displayPath } from "../turn-diff-files";
import {
  isAbsoluteWorkspacePath,
  normalizeDiffPath,
  pathsMatchForWorkspace,
  stripTaskCwdPrefix,
} from "../../../lib/workspace-path";
import type { SessionArtifactIndex } from "./artifact-index";
import { isPathLikeToken, parseChatHref, parsePathCandidate } from "./parse";

export type ChatLinkClassification =
  | { kind: "none" }
  | { kind: "web"; href: string }
  | {
      kind: "diff" | "files" | "directory" | "artifact";
      path: string;
      line: number | undefined;
      column: number | undefined;
      displayPath: string;
    };

export interface ClassifyChatCandidateInput {
  source: "inline-code" | "href";
  raw: string;
  index: SessionArtifactIndex;
  hasNavigation: boolean;
  cwd?: string | null;
}

interface PathCollectionLookup {
  exact: Map<string, string>;
  basenames: Map<string, string[]>;
}

const pathCollectionCache = new WeakMap<string[], PathCollectionLookup>();
const combinedBasenameCache = new WeakMap<
  SessionArtifactIndex,
  Map<string, string[]>
>();

/** Last path segment after slash normalization, used for unique bare-filename hits. */
function basename(path: string): string {
  return normalizeDiffPath(path).split("/").at(-1) ?? "";
}

/** Builds exact and basename indexes once for each immutable artifact array. */
function pathCollectionLookup(entries: string[]): PathCollectionLookup {
  const cached = pathCollectionCache.get(entries);
  if (cached !== undefined) return cached;
  const exact = new Map<string, string>();
  const basenames = new Map<string, string[]>();
  for (const entry of entries) {
    const normalized = normalizeDiffPath(displayPath(entry))
      .replace(/\/+$/, "")
      .toLowerCase();
    exact.set(normalized, entry);
    const name = basename(entry).toLowerCase();
    const hits = basenames.get(name) ?? [];
    hits.push(entry);
    basenames.set(name, hits);
  }
  const lookup = { exact, basenames };
  pathCollectionCache.set(entries, lookup);
  return lookup;
}

/**
 * Resolves a typed path against one index collection. Bare filenames skip exact
 * equality so a root `README.md` cannot hide another `…/README.md`.
 */
export function matchIndexPath(
  candidate: string,
  entries: string[],
): string | null {
  const normalized = normalizeDiffPath(displayPath(candidate)).replace(
    /\/+$/,
    "",
  );
  if (!normalized.includes("/")) {
    const target = normalized.toLowerCase();
    const hits = pathCollectionLookup(entries).basenames.get(target) ?? [];
    return hits.length === 1 ? hits[0]! : null;
  }

  const exact = pathCollectionLookup(entries).exact.get(
    normalized.toLowerCase(),
  );
  if (exact !== undefined) return exact;

  const suffixHits = entries.filter((entry) =>
    pathsMatchForWorkspace(entry, normalized),
  );
  return suffixHits.length === 1 ? suffixHits[0]! : null;
}

/**
 * Bare filenames must be unique across edited and referenced together, except a
 * workspace-root file (README.md) may win over nested copies of the same name.
 */
function uniqueBasenameHit(
  candidate: string,
  index: SessionArtifactIndex,
  cwd?: string | null,
): string | null {
  const target = normalizeDiffPath(displayPath(candidate)).toLowerCase();
  if (target.includes("/") || target === "") return null;
  let combined = combinedBasenameCache.get(index);
  if (combined === undefined) {
    combined = new Map<string, string[]>();
    for (const entry of [...index.edited, ...index.referenced]) {
      const name = basename(entry).toLowerCase();
      const hits = combined.get(name) ?? [];
      hits.push(entry);
      combined.set(name, hits);
    }
    combinedBasenameCache.set(index, combined);
  }
  const hits = combined.get(target) ?? [];
  if (hits.length <= 1) return hits[0] ?? null;

  const relativeRoots = hits.filter((hit) => {
    const relative = toNavigationPath(hit, cwd);
    return relative !== null && relative.toLowerCase() === target;
  });
  if (relativeRoots.length === 1) return relativeRoots[0]!;
  return uniqueShallowestHit(hits);
}

/**
 * When several files share a basename, prefer the unique ancestor whose parent
 * directory prefixes every other hit (workspace-root README.md over nested copies).
 */
function uniqueShallowestHit(hits: string[]): string | null {
  const roots = hits.filter((hit) => {
    const normalizedHit = normalizeDiffPath(displayPath(hit)).toLowerCase();
    const dir = parentDir(normalizedHit);
    return hits.every((other) => {
      const normalizedOther = normalizeDiffPath(
        displayPath(other),
      ).toLowerCase();
      if (normalizedOther === normalizedHit) return true;
      if (dir === "") return true;
      return normalizedOther.startsWith(`${dir}/`);
    });
  });
  return roots.length === 1 ? roots[0]! : null;
}

/** Parent directory after slash normalization, or empty for a bare filename. */
function parentDir(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash === -1 ? "" : path.slice(0, slash);
}

/**
 * Converts an index hit (which may still be absolute) into the workspace-relative
 * path Files and Diff accept. Returns null when the hit is outside the task cwd
 * so a suffix match cannot open a different relative file inside the worktree.
 */
export function toNavigationPath(
  storedPath: string,
  cwd: string | null | undefined,
): string | null {
  const normalized = normalizeDiffPath(displayPath(storedPath));
  if (normalized.split("/").includes("..")) return null;
  if (cwd !== null && cwd !== undefined && cwd !== "") {
    const stripped =
      stripTaskCwdPrefix(storedPath, cwd) ??
      stripTaskCwdPrefix(normalized, cwd);
    if (
      stripped !== null &&
      stripped !== "" &&
      !isAbsoluteWorkspacePath(stripped)
    ) {
      return stripped;
    }
    if (
      isAbsoluteWorkspacePath(storedPath) ||
      isAbsoluteWorkspacePath(normalized)
    ) {
      return null;
    }
  }
  if (
    !isAbsoluteWorkspacePath(storedPath) &&
    !isAbsoluteWorkspacePath(normalized)
  ) {
    return normalized;
  }
  return null;
}

/** Classifies one inline-code token or Markdown href against the session artifact index. */
export function classifyChatCandidate(
  input: ClassifyChatCandidateInput,
): ChatLinkClassification {
  if (!input.hasNavigation) return { kind: "none" };

  if (input.source === "href") {
    const parsed = parseChatHref(input.raw);
    if (parsed.kind === "web") return parsed;
    if (parsed.kind === "inert") return { kind: "none" };
    return classifyFileCandidate(
      parsed.path,
      parsed.line,
      parsed.column,
      input,
      true,
    );
  }

  const href = parseChatHref(input.raw);
  if (href.kind === "web") return href;
  const parsed = parsePathCandidate(input.raw);
  if (
    !isPathLikeToken(input.raw) &&
    matchIndexPath(parsed.path, input.index.directories ?? []) === null &&
    matchIndexPath(parsed.path, input.index.unknown ?? []) === null &&
    matchIndexPath(parsed.path, input.index.edited) === null &&
    matchIndexPath(parsed.path, input.index.referenced) === null
  ) {
    return { kind: "none" };
  }
  return classifyFileCandidate(
    parsed.path,
    parsed.line,
    parsed.column,
    input,
    false,
  );
}

/** Routes a file candidate to Diff, Files, or none. Href misses still attempt Files. */
function classifyFileCandidate(
  path: string,
  line: number | undefined,
  column: number | undefined,
  input: ClassifyChatCandidateInput,
  hrefMissOpensFiles: boolean,
): ChatLinkClassification {
  const normalized = normalizeDiffPath(displayPath(path));
  const fileHit = normalized.includes("/")
    ? (matchIndexPath(path, input.index.edited) ??
      matchIndexPath(path, input.index.referenced))
    : uniqueBasenameHit(path, input.index, input.cwd);
  if (fileHit !== null) {
    const fileKey = normalizeDiffPath(displayPath(fileHit))
      .replace(/\/+$/, "")
      .toLowerCase();
    const kind = pathCollectionLookup(input.index.edited).exact.has(fileKey)
      ? "diff"
      : "files";
    return navigationClassification(kind, fileHit, line, column, input);
  }

  const directoryHit = matchIndexPath(path, input.index.directories ?? []);
  if (directoryHit !== null) {
    return navigationClassification(
      "directory",
      directoryHit,
      /*line*/ undefined,
      /*column*/ undefined,
      input,
    );
  }

  const unknownHit = matchIndexPath(path, input.index.unknown ?? []);
  if (unknownHit !== null) {
    return navigationClassification(
      "artifact",
      unknownHit,
      line,
      column,
      input,
    );
  }

  if (!hrefMissOpensFiles) return { kind: "none" };
  const explicitDirectory = /[\\/]$/.test(path);
  return navigationClassification(
    explicitDirectory ? "directory" : "files",
    path,
    line,
    column,
    input,
  );
}

/** Drops candidates whose stored path cannot be opened as a worktree-relative file. */
function navigationClassification(
  kind: "diff" | "files" | "directory" | "artifact",
  storedPath: string,
  line: number | undefined,
  column: number | undefined,
  input: ClassifyChatCandidateInput,
): ChatLinkClassification {
  const rootDirectory =
    kind === "directory" &&
    input.cwd !== null &&
    input.cwd !== undefined &&
    stripTaskCwdPrefix(storedPath, input.cwd) === "";
  const navigationPath = rootDirectory
    ? ""
    : toNavigationPath(storedPath, input.cwd);
  if (navigationPath === null) return { kind: "none" };
  return {
    kind,
    path: navigationPath,
    line,
    column,
    displayPath: storedPath,
  };
}
