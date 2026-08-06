import type { WorkflowArtifact } from "@ora/workflow-runtime";

/** Filter mode: all run artifacts or one node's outputs. */
export type ArtifactFilterMode =
  | { type: "all" }
  | { type: "node"; nodeId: string };

/**
 * Returns artifacts for a scope, newest first.
 * Node filter keeps only that node's outputs; empty filters still return [].
 */
export function filterArtifacts(
  artifacts: readonly WorkflowArtifact[],
  mode: ArtifactFilterMode,
): WorkflowArtifact[] {
  const scoped = mode.type === "all"
    ? [...artifacts]
    : artifacts.filter((item) => item.nodeId === mode.nodeId);
  return scoped.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}

/** Newest artifact by createdAt, or null when the list is empty. */
export function latestArtifact(
  artifacts: readonly WorkflowArtifact[],
): WorkflowArtifact | null {
  if (artifacts.length === 0) {
    return null;
  }
  return filterArtifacts(artifacts, { type: "all" })[0] ?? null;
}
