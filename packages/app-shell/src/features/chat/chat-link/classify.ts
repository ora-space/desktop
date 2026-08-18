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
 * Resolves a typed path against the session index. Bare filenames link only when
 * exactly one index entry shares that last segment; the hit's stored path is returned.
 */
export function matchIndexPath(
  candidate: string,
  entries: string[],
): string | null {
  const normalized = normalizeDiffPath(displayPath(candidate));
  const exact = entries.find(
    (entry) => entry.toLowerCase() === normalized.toLowerCase(),
  );
  if (exact !== undefined) return exact;

  if (!normalized.includes("/")) {
    const target = normalized.toLowerCase();
    const hits = entries.filter(
      (entry) => basename(entry).toLowerCase() === target,
    );
    return hits.length === 1 ? hits[0]! : null;
  }

  const suffixHits = entries.filter((entry) =>
    pathsMatchForWorkspace(entry, normalized),
  );
  return suffixHits.length === 1 ? suffixHits[0]! : null;
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
  const editedHit = matchIndexPath(path, input.index.edited);
  const referencedHit =
    editedHit === null ? matchIndexPath(path, input.index.referenced) : null;
  const hit = editedHit ?? referencedHit;
  if (hit === null && !hrefMissOpensFiles) return { kind: "none" };

  const navigationPath = toNavigationPath(hit ?? path, input.cwd);
  if (navigationPath === null) return { kind: "none" };
  const kind = editedHit !== null ? "diff" : "files";
  return {
    kind,
    path: navigationPath,
    line,
    column,
    displayPath: hit ?? path,
  };
}
