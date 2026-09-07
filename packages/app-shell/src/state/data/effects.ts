/** Cache identity owned by effects data; consumers never repeat its tuples. */
export const effectKeys = {
  agentEffectStatus: (workspaceId: string, agentRef: string) =>
    ["agent-effect-status", workspaceId, agentRef] as const,
};
