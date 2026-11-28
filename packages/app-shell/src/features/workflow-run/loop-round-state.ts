import type {
  GraphWorkflowNodeState,
  GraphWorkflowRound,
  GraphWorkflowRun,
} from "@ora/workflow-runtime";

export type LoopRoundSelection = Readonly<Record<string, string>>;

/** Distinguish repeated round numbers when history spans several container executions. */
export function loopRoundLabel(
  ordinal: string,
  round: GraphWorkflowRound,
  showAttempt: boolean,
) {
  return showAttempt ? `${ordinal} · ${round.parentLoopNodeRunId}` : ordinal;
}

/** Container history stays pinned when a background occurrence completes. */
export function isLoopHistoryNode(
  run: GraphWorkflowRun,
  nodeId: string,
): boolean {
  const node = run.definitionSnapshot.nodes.find((node) => node.id === nodeId);
  return (
    node?.data.kind === "loop" ||
    run.definitionSnapshot.nodes.some(
      (owner) =>
        owner.data.kind === "loop" &&
        owner.id === (node?.data.containerId ?? node?.parentId),
    )
  );
}

/** Presents committed round facts without re-evaluating conditions or guessing missing values. */
export function loopRoundOutcome(
  run: GraphWorkflowRun,
  round: GraphWorkflowRound,
): string {
  if (round.status === "failed") return "workflowRun.loopView.failed";
  if (round.status === "cancelled") return "workflowRun.loopView.cancelled";
  if (
    Object.values(round.nodeStates).some(
      (state) => state.status === "awaiting_input",
    )
  )
    return "workflowRun.loopView.waiting";
  if (round.status === "running" || round.status === "pending") {
    const exiting = run.definitionSnapshot.nodes.some(
      (node) =>
        node.data.kind === "loopExit" &&
        round.nodeStates[node.id]?.status === "succeeded",
    );
    return exiting
      ? "workflowRun.loopView.finishing"
      : "workflowRun.loopView.running";
  }
  if (
    (run.rounds ?? []).some(
      (other) =>
        other.parentLoopNodeRunId === round.parentLoopNodeRunId &&
        other.roundIndex > round.roundIndex,
    )
  )
    return "workflowRun.loopView.continue";
  const parent = run.nodeStates[round.parentLoopNodeId];
  if (parent?.nodeRunId !== round.parentLoopNodeRunId)
    return "workflowRun.loopView.completed";
  if (parent?.stopReason === "loop_max_iterations")
    return "workflowRun.loopView.maxIterations";
  if (parent?.stopReason === "loop_exit") return "workflowRun.loopView.exit";
  if (parent?.stopReason === "loop_succeeded")
    return "workflowRun.loopView.condition";
  return "workflowRun.loopView.completed";
}

/** Resolves one Loop's explicit round selection, defaulting to the first round of the current container execution. */
export function selectedLoopRound(
  run: GraphWorkflowRun,
  loopNodeId: string,
  selection: LoopRoundSelection,
): GraphWorkflowRound | undefined {
  const currentAttempt = run.nodeStates[loopNodeId]?.nodeRunId;
  const loopRounds = (run.rounds ?? []).filter(
    (round) => round.parentLoopNodeId === loopNodeId,
  );
  const selectedRoundId = selection[loopNodeId];
  return (
    loopRounds.find((round) => round.id === selectedRoundId) ??
    loopRounds
      .filter(
        (round) =>
          !currentAttempt || round.parentLoopNodeRunId === currentAttempt,
      )
      .reduce<GraphWorkflowRound | undefined>(
        (latest, round) =>
          latest === undefined || round.roundIndex < latest.roundIndex
            ? round
            : latest,
        undefined,
      )
  );
}

/**
 * Projects one selected round per Loop onto the Theater's flat node-state view.
 * Persisted child runs stay isolated in round history; this projection only decides
 * which occurrence is visible in cards, path status, focus, and session details.
 */
export function projectLoopRoundNodeStates(
  run: GraphWorkflowRun,
  selection: LoopRoundSelection,
): Record<string, GraphWorkflowNodeState> {
  const loopNodeIds = new Set(
    run.definitionSnapshot.nodes
      .filter((node) => node.data.kind === "loop")
      .map((node) => node.id),
  );
  const nodeStates = { ...run.nodeStates };
  for (const loopNodeId of loopNodeIds) {
    // A member absent from this round must never inherit another occurrence's session/output.
    for (const member of run.definitionSnapshot.nodes) {
      if ((member.data.containerId ?? member.parentId) === loopNodeId) {
        nodeStates[member.id] = { status: "inactive" };
      }
    }
    const round = selectedLoopRound(run, loopNodeId, selection);
    if (round !== undefined) {
      Object.assign(nodeStates, round.nodeStates);
    }
  }
  return nodeStates;
}
