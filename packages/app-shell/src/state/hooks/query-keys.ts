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
};

export type WorkspaceQueryKey = readonly ["projects"] | readonly ["tasks"] | readonly ["sessions"];
