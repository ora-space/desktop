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
      kind: "diff" | "files";
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

/** Last path segment after slash normalization, used for unique bare-filename hits. */
function basename(path: string): string {
  return normalizeDiffPath(path).split("/").at(-1) ?? "";
}

/**
 * Resolves a typed path against one index collection. Bare filenames skip exact
 * equality so a root `README.md` cannot hide another `…/README.md`.
 */
export function matchIndexPath(
  candidate: string,
  entries: string[],
): string | null {
  const normalized = normalizeDiffPath(displayPath(candidate));
  if (!normalized.includes("/")) {
    const target = normalized.toLowerCase();
    const hits = entries.filter(
      (entry) => basename(entry).toLowerCase() === target,
    );
    return hits.length === 1 ? hits[0]! : null;
  }

  const exact = entries.find(
    (entry) => entry.toLowerCase() === normalized.toLowerCase(),
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
  const hits = [...index.edited, ...index.referenced].filter(
    (entry) => basename(entry).toLowerCase() === target,
  );
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
  if (!isPathLikeToken(input.raw)) return { kind: "none" };
  const parsed = parsePathCandidate(input.raw);
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
  if (!normalized.includes("/")) {
    const unique = uniqueBasenameHit(path, input.index, input.cwd);
    if (unique === null) {
      if (!hrefMissOpensFiles) return { kind: "none" };
      return navigationClassification("files", path, line, column, input);
    }
    const kind = input.index.edited.some(
      (entry) => entry.toLowerCase() === unique.toLowerCase(),
    )
      ? "diff"
      : "files";
    return navigationClassification(kind, unique, line, column, input);
  }

  const editedHit = matchIndexPath(path, input.index.edited);
  const referencedHit =
    editedHit === null ? matchIndexPath(path, input.index.referenced) : null;
  const hit = editedHit ?? referencedHit;
  if (hit === null && !hrefMissOpensFiles) return { kind: "none" };

  const kind = editedHit !== null ? "diff" : "files";
  return navigationClassification(kind, hit ?? path, line, column, input);
}

/** Drops candidates whose stored path cannot be opened as a worktree-relative file. */
function navigationClassification(
  kind: "diff" | "files",
  storedPath: string,
  line: number | undefined,
  column: number | undefined,
  input: ClassifyChatCandidateInput,
): ChatLinkClassification {
  const navigationPath = toNavigationPath(storedPath, input.cwd);
  if (navigationPath === null) return { kind: "none" };
  return {
    kind,
    path: navigationPath,
    line,
    column,
    displayPath: storedPath,
  };
}
