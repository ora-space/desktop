/**
 * Shared slash-normalization and matching for chat links and the Changes panel.
 *
 * Chat classification and Diff file requests must use the same rules so a bottom
 * summary click and an inline path cannot disagree about the same artifact.
 */

/** Converts provider and Git path styles before matching a chat file to a task patch. */
export function normalizeDiffPath(path: string): string {
  return path.replaceAll("\\", "/").replace(/^\.?\//, "");
}

/** True when the token is a Windows drive path or a POSIX rooted path. */
export function isAbsoluteWorkspacePath(path: string): boolean {
  const slashed = path.replaceAll("\\", "/");
  return slashed.startsWith("/") || /^[A-Za-z]:\//.test(slashed);
}

/**
 * Bidirectional suffix match used by Changes requests and chat-link classification.
 * Comparison is case-insensitive so Windows worktrees and project-root checkouts agree.
 */
export function pathsMatchForWorkspace(left: string, right: string): boolean {
  const a = normalizeDiffPath(left).toLowerCase();
  const b = normalizeDiffPath(right).toLowerCase();
  return a === b || a.endsWith(`/${b}`) || b.endsWith(`/${a}`);
}

/**
 * Strips a task cwd prefix from an ACP path so Files/Diff receive a workspace-relative
 * path. Returns null when the path is not under that cwd.
 */
export function stripTaskCwdPrefix(path: string, cwd: string): string | null {
  const normalizedPath = normalizeDiffPath(path);
  const normalizedCwd = normalizeDiffPath(cwd).replace(/\/+$/, "");
  if (normalizedCwd === "") return null;
  const pathLower = normalizedPath.toLowerCase();
  const cwdLower = normalizedCwd.toLowerCase();
  if (pathLower === cwdLower) return "";
  const prefix = `${cwdLower}/`;
  if (!pathLower.startsWith(prefix)) return null;
  return normalizedPath.slice(normalizedCwd.length + 1);
}

/**
 * Builds the OS-absolute path for Explorer / VS Code / copy. Absolute tool paths
 * are kept; relative paths are joined onto the resolved task cwd with forward slashes.
 */
export function joinOsAbsolutePath(path: string, cwd: string): string {
  if (isAbsoluteWorkspacePath(path)) return path.replaceAll("\\", "/");
  const root = cwd.replaceAll("\\", "/").replace(/\/+$/, "");
  const relative = normalizeDiffPath(path);
  return relative === "" ? root : `${root}/${relative}`;
}
