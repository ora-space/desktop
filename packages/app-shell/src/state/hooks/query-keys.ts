import type { TaskDiffScope, WarmSessionTarget } from "@ora/contracts";

/**
 * Centralised react-query cache keys for the app shell.
 *
 * Keeping keys in one place lets mutations invalidate exactly the queries they
 * share data with, without scattering string literals across hook files.
 */
export const queryKeys = {
  projects: ["projects"] as const,
  projectBranches: (projectId: string) =>
    ["project-branches", projectId] as const,
  tasks: ["tasks"] as const,
  sessions: ["sessions"] as const,
  agents: ["agents"] as const,
  skills: ["skills"] as const,
  availablePlugins: ["available-plugins"] as const,
  installedPlugins: ["installed-plugins"] as const,
  developerMode: ["developer-mode"] as const,
  runtimeLogLevel: ["runtime-log-level"] as const,
  gitIdentity: ["gitIdentity"] as const,
  /** Project → mounted graph workflow definitions (mock host). */
  workflowMounts: (projectId: string) => ["workflowMounts", projectId] as const,
  /** Definition → projects that already mount it. */
  workflowMountsByDefinition: (definitionId: string) =>
    ["workflowMountsByDefinition", definitionId] as const,
  /** Project → GraphWorkflowRun list (mock run repo). */
  workflowRuns: (projectId: string) => ["workflowRuns", projectId] as const,
  workflowRun: (runId: string) => ["workflowRun", runId] as const,
  /** Artifacts produced by one graph workflow run. */
  workflowArtifacts: (runId: string) => ["workflowArtifacts", runId] as const,
  agentRuntimeStatus: ["agentRuntimeStatus"] as const,
  taskWorkspace: (taskId: string) => ["task-workspace", taskId] as const,
  taskDiffs: (taskId: string) => ["task-diff", taskId] as const,
  taskDiff: (taskId: string, scope: TaskDiffScope) =>
    ["task-diff", taskId, scope] as const,
  taskDiffComments: (taskId: string) => ["task-diff-comments", taskId] as const,
  workspaceFiles: (taskId: string) => ["workspace-files", taskId] as const,
  workspaceDirectory: (taskId: string, path: string) =>
    ["workspace-files", taskId, "directory", path] as const,
  workspaceFile: (taskId: string, path: string) =>
    ["workspace-files", taskId, "file", path] as const,
  workspaceSearch: (taskId: string, kind: string, query: string) =>
    ["workspace-files", taskId, "search", kind, query] as const,
  projectDirectory: (projectId: string, path: string) =>
    ["project-files", projectId, "directory", path] as const,
  projectSearch: (projectId: string, kind: string, query: string) =>
    ["project-files", projectId, "search", kind, query] as const,
  /**
   * Mirrors the identity the backend keys warm sessions by, so two surfaces
   * never share one cache entry and revisiting a surface reuses its session.
   */
  warmSession: (target: WarmSessionTarget | null, agentRef: string) =>
    [
      "warmSession",
      target?.type ?? "none",
      targetId(target),
      agentRef,
    ] as const,
  specs: (projectId: string) => ["specs", projectId] as const,
  specCatalog: (projectId: string, targetKey: string) =>
    ["specs", projectId, "catalog", targetKey] as const,
  specDocument: (projectId: string, targetKey: string, path: string) =>
    ["specs", projectId, "document", targetKey, path] as const,
};

/** Extracts the identifier a warm target is scoped to, for cache-key purposes. */
function targetId(target: WarmSessionTarget | null): string {
  if (target === null) return "";
  return target.type === "task" ? target.taskId : target.projectId;
}

export type WorkspaceQueryKey =
  readonly ["projects"] | readonly ["tasks"] | readonly ["sessions"];
