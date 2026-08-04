import type { TaskDiffScope } from "@ora/contracts";

/**
 * Centralised react-query cache keys for the app shell.
 *
 * Keeping keys in one place lets mutations invalidate exactly the queries they
 * share data with, without scattering string literals across hook files.
 */
export const queryKeys = {
  projects: ["projects"] as const,
  projectBranches: (projectId: string) => ["project-branches", projectId] as const,
  tasks: ["tasks"] as const,
  sessions: ["sessions"] as const,
  agents: ["agents"] as const,
  skills: ["skills"] as const,
  gitIdentity: ["gitIdentity"] as const,
  agentModels: ["agentModels"] as const,
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
  taskWorkspace: (taskId: string) => ["task-workspace", taskId] as const,
  taskDiffs: (taskId: string) => ["task-diff", taskId] as const,
  taskDiff: (taskId: string, scope: TaskDiffScope) => ["task-diff", taskId, scope] as const,
  taskDiffComments: (taskId: string) => ["task-diff-comments", taskId] as const,
};

export type WorkspaceQueryKey = readonly ["projects"] | readonly ["tasks"] | readonly ["sessions"];
