import type {
  GraphWorkflowRun,
  WorkflowNodeCompletion,
} from "@ora/workflow-runtime";

/**
 * True when the workflow's opening turn settled normally but the user has not confirmed the step.
 *
 * The node is awaiting the user rather than paused: its only blocker is `turn_unconfirmed`, so the
 * step card keeps the awaiting-input badge and offers continue / complete-anyway instead of reading
 * as a turn that ended abnormally.
 */
export function isAwaitingConfirmation(
  completion: WorkflowNodeCompletion | undefined,
): boolean {
  if (
    completion === undefined ||
    completion.ready ||
    completion.provisional ||
    completion.blockers.length === 0
  ) {
    return false;
  }
  return completion.blockers.every(
    (blocker) => blocker.kind === "turn_unconfirmed",
  );
}

/**
 * Direct successor titles for every node in one frozen graph, keyed by node id.
 *
 * The completion card names where an interactive step advances to. Titles follow edge order and
 * are deduplicated; an empty list means the node is the last step, so the card must not claim a
 * next step that does not exist.
 */
export function directSuccessorNodeTitles(
  run: GraphWorkflowRun,
): Record<string, string[]> {
  const titleById = new Map(
    run.definitionSnapshot.nodes.map((node) => [node.id, node.data.title]),
  );
  const nextByNodeId: Record<string, string[]> = {};
  for (const edge of run.definitionSnapshot.edges) {
    const target = titleById.get(edge.target);
    if (target === undefined) {
      continue;
    }
    const titles = nextByNodeId[edge.source] ?? [];
    if (!titles.includes(target)) {
      titles.push(target);
    }
    nextByNodeId[edge.source] = titles;
  }
  return nextByNodeId;
}
