import type { TaskDiffScope } from "@ora/contracts";

/**
 * Centralised react-query cache keys for the app shell.
 *
 * Keeping keys in one place lets mutations invalidate exactly the queries they
 * share data with, without scattering string literals across hook files.
 */
export const queryKeys = {
  projects: ["projects"] as const,
  tasks: ["tasks"] as const,
  sessions: ["sessions"] as const,
  agents: ["agents"] as const,
  skills: ["skills"] as const,
  gitIdentity: ["gitIdentity"] as const,
  agentModels: ["agentModels"] as const,
  taskWorkspace: (taskId: string) => ["task-workspace", taskId] as const,
  taskDiffs: (taskId: string) => ["task-diff", taskId] as const,
  taskDiff: (taskId: string, scope: TaskDiffScope) => ["task-diff", taskId, scope] as const,
  taskDiffComments: (taskId: string) => ["task-diff-comments", taskId] as const,
};

export type WorkspaceQueryKey = readonly ["projects"] | readonly ["tasks"] | readonly ["sessions"];
