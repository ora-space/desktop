import type { WorkspaceFileChange } from "@ora/contracts";
import type { QueryClient } from "@tanstack/react-query";
import { queryKeys } from "../../state/hooks/query-keys";

/** Resolves a Markdown link only when it remains a workspace-relative Markdown path. */
export function resolveMarkdownLink(currentPath: string, href: string): string | null {
  const clean = href.split(/[?#]/u, 1)[0] ?? "";
  if (!/\.mdx?$/iu.test(clean) || clean.startsWith("/")) return null;
  const segments = [...currentPath.split("/").slice(0, -1), ...clean.split("/")];
  const normalized: string[] = [];
  for (const segment of segments) {
    if (segment === "" || segment === ".") continue;
    if (segment === "..") {
      if (normalized.length === 0) return null;
      normalized.pop();
    } else {
      normalized.push(segment);
    }
  }
  return normalized.join("/");
}

/** Invalidates document content narrowly while refreshing catalogs after structural changes. */
export function invalidateSpecQueries(
  queryClient: QueryClient,
  projectId: string,
  targetKey: string,
  changes: WorkspaceFileChange[],
) {
  let invalidateCatalog = false;
  for (const change of changes) {
    if (change.kind === "modified" && /\.mdx?$/iu.test(change.path)) {
      void queryClient.invalidateQueries({ queryKey: queryKeys.specDocument(projectId, targetKey, change.path) });
      continue;
    }
    if (change.kind === "rescanRequired" || change.kind === "renamed" || change.path.endsWith(".gitignore") || /\.mdx?$/iu.test(change.path)) {
      invalidateCatalog = true;
    }
  }
  if (invalidateCatalog) void queryClient.invalidateQueries({ queryKey: queryKeys.specCatalog(projectId, targetKey) });
}
