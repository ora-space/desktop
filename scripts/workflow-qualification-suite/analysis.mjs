/** Validates all generated documents through the actual engine without persisting any workflows. */
export async function analyzeWorkflows(session, workflows, { signal } = {}) {
  const analyses = [];
  for (const workflow of workflows) {
    if (signal?.aborted) throw new Error("Qualification interrupted.");
    try {
      const response = await session.invoke("analyze_workflow", {
        graph: JSON.stringify(workflow.graph),
      });
      if (
        !Array.isArray(response?.unusedNodeIds) ||
        response.unusedNodeIds.some((id) => typeof id !== "string")
      ) {
        throw new Error("analyze_workflow returned invalid unusedNodeIds.");
      }
      analyses.push({
        index: workflow.index,
        name: workflow.name,
        valid: true,
        unusedNodeIds: response.unusedNodeIds,
      });
    } catch (error) {
      analyses.push({
        index: workflow.index,
        name: workflow.name,
        valid: false,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }
  return analyses;
}
