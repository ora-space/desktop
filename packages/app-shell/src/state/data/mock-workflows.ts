/** Cache identity owned by mock-workflows data; consumers never repeat its tuples. */
export const mockWorkflowKeys = {
  workflowMounts: (projectId: string) => ["workflowMounts", projectId] as const,
  workflowMountsByDefinition: (definitionId: string) =>
    ["workflowMountsByDefinition", definitionId] as const,
  workflowRuns: (projectId: string) => ["workflowRuns", projectId] as const,
  workflowRun: (runId: string) => ["workflowRun", runId] as const,
  workflowArtifacts: (runId: string) => ["workflowArtifacts", runId] as const,
};
