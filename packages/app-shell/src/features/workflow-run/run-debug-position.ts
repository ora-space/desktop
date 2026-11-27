import type { PendingWorkflowDebugTarget } from "@ora/contracts";
import type {
  GraphWorkflowRound,
  GraphWorkflowRun,
  WorkflowDefinitionNode,
} from "@ora/workflow-runtime";

/** Includes the bound item even before its first member acquires an execution record. */
export function iterationItemIndices(
  run: GraphWorkflowRun,
  owner: string,
  memberIds: string[],
) {
  const indices = new Set<number>();
  const bound = run.nodeStates[owner]?.boundItemIndex;
  if (bound !== undefined) indices.add(bound);
  for (const id of memberIds)
    for (const state of run.roundStates?.[id] ?? [])
      if (state.iteration !== undefined) indices.add(state.iteration);
  return [...indices].sort((left, right) => left - right);
}

/** Names the visible position for the backend to check against its live scheduler projection. */
export function pendingDebugPosition(
  run: GraphWorkflowRun,
  node: WorkflowDefinitionNode | undefined,
  round: GraphWorkflowRound | undefined,
  item: number | null,
): PendingWorkflowDebugTarget | undefined {
  const rootScopeId = run.rootScopeId;
  if (!rootScopeId || !node) return;
  const position = { rootScopeId, scopeId: rootScopeId, nodeId: node.id };
  const owner = node.data.containerId ?? node.parentId;
  if (!owner) return position;
  if (round?.parentLoopNodeId === owner)
    return {
      ...position,
      scopeId: round.id,
      containerNodeRunId: round.parentLoopNodeRunId,
    };
  const container = run.definitionSnapshot.nodes.find(
    (candidate) => candidate.id === owner,
  );
  const containerNodeRunId = run.nodeStates[owner]?.nodeRunId;
  if (
    container?.data.kind === "iteration" &&
    containerNodeRunId &&
    item !== null
  )
    return { ...position, containerNodeRunId, index: item };
}
