import type { GraphWorkflowNodeStatus, GraphWorkflowRun } from "@ora/workflow-runtime";
import { isTerminalRunStatus } from "./run-status-style";

const ACTIVE_STATUSES: ReadonlySet<GraphWorkflowNodeStatus> = new Set([
  "running",
  "awaiting_input",
]);

const TERMINAL_NODE_STATUSES: ReadonlySet<GraphWorkflowNodeStatus> = new Set([
  "succeeded",
  "failed",
  "cancelled",
]);

export interface TheaterFocus {
  /** Primary act shown large on stage. */
  primaryId: string | null;
  /**
   * All currently active acts (running + awaiting_input), snapshot order.
   * Length > 1 means genuine parallelism from the UI's point of view.
   */
  activeIds: string[];
}

/** Last observed status for the Theater preferred focus (edge detection). */
export interface TheaterFocusStatusSample {
  nodeId: string;
  status: GraphWorkflowNodeStatus;
}

function isActiveStatus(status: GraphWorkflowNodeStatus): boolean {
  return ACTIVE_STATUSES.has(status);
}

function isTerminalNodeStatus(status: GraphWorkflowNodeStatus): boolean {
  return TERMINAL_NODE_STATUSES.has(status);
}

/**
 * True when the same focused act just left live —terminal.
 * Clears a live pin so Theater can resume auto-follow; history pins
 * (clicked while already non-live) must not release.
 */
export function shouldReleaseFocusToFollow(
  prev: TheaterFocusStatusSample | null,
  focusNodeId: string | null,
  currentStatus: GraphWorkflowNodeStatus | undefined,
): boolean {
  if (
    prev === null
    || focusNodeId === null
    || currentStatus === undefined
    || prev.nodeId !== focusNodeId
  ) {
    return false;
  }
  return isActiveStatus(prev.status) && isTerminalNodeStatus(currentStatus);
}

/** Snapshot order keeps parallel chips stable across engine patches. */
function orderedActiveIds(run: GraphWorkflowRun): string[] {
  return run.definitionSnapshot.nodes
    .map((node) => node.id)
    .filter((nodeId) => {
      const state = run.nodeStates[nodeId];
      return state !== undefined && isActiveStatus(state.status);
    });
}

/**
 * Among active acts, prefer awaiting_input, then latest startedAt, then snapshot order.
 */
function pickPrimaryAmongActive(
  run: GraphWorkflowRun,
  activeIds: string[],
): string | null {
  if (activeIds.length === 0) {
    return null;
  }

  const awaiting = activeIds.filter(
    (nodeId) => run.nodeStates[nodeId]?.status === "awaiting_input",
  );
  const pool = awaiting.length > 0 ? awaiting : activeIds;

  let bestId = pool[0]!;
  let bestStarted = run.nodeStates[bestId]?.startedAt ?? "";
  for (const nodeId of pool.slice(1)) {
    const started = run.nodeStates[nodeId]?.startedAt ?? "";
    if (started.localeCompare(bestStarted) > 0) {
      bestId = nodeId;
      bestStarted = started;
    }
  }
  return bestId;
}

function pickFallbackPrimary(run: GraphWorkflowRun): string | null {
  let latestSucceeded: { nodeId: string; finishedAt: string } | null = null;
  for (const node of run.definitionSnapshot.nodes) {
    const state = run.nodeStates[node.id];
    if (state?.status === "succeeded" && state.finishedAt !== undefined) {
      if (
        latestSucceeded === null
        || state.finishedAt.localeCompare(latestSucceeded.finishedAt) > 0
      ) {
        latestSucceeded = { nodeId: node.id, finishedAt: state.finishedAt };
      }
    }
  }
  if (latestSucceeded !== null) {
    return latestSucceeded.nodeId;
  }
  return run.definitionSnapshot.nodes[0]?.id ?? null;
}

/**
 * Resolves Theater spotlight under sequential or parallel execution.
 *
 * - `activeIds`: every running / awaiting_input node (may be many).
 * - `primaryId`: user preference if still valid; else policy among actives;
 *   else last succeeded / first node.
 * Live pins are released by the workspace when the focused act just finishes
 * (`shouldReleaseFocusToFollow`); history pins stay sticky here.
 */
export function resolveTheaterFocus(
  run: GraphWorkflowRun,
  preferredNodeId: string | null,
): TheaterFocus {
  const activeIds = orderedActiveIds(run);

  if (
    preferredNodeId !== null
    && run.nodeStates[preferredNodeId] !== undefined
  ) {
    // Keep user focus even after the act leaves "active", until they pick another
    // or the workspace releases a live pin that just finished.
    return { primaryId: preferredNodeId, activeIds };
  }

  const primaryId = pickPrimaryAmongActive(run, activeIds)
    ?? pickFallbackPrimary(run);
  return { primaryId, activeIds };
}

/**
 * Overview selection ring. Terminal + no pin stays unselected so Theater's
 * result act and the graph do not disagree on "who is focused".
 */
export function resolveOverviewFocusedId(
  run: GraphWorkflowRun,
  focusedNodeId: string | null,
): string | null {
  if (focusedNodeId === null && isTerminalRunStatus(run.status)) {
    return null;
  }
  return resolveTheaterFocus(run, focusedNodeId).primaryId;
}

/**
 * Effective stage focus: an open node session wins over path / auto-follow pins.
 */
export function resolveStageFocusNodeId(
  conversationNodeId: string | null,
  focusNodeId: string | null,
): string | null {
  return conversationNodeId ?? focusNodeId;
}

/**
 * Live-pin release is suppressed while a node session owns attention.
 */
export function shouldReleaseLivePinToFollow(
  conversationNodeId: string | null,
  prev: TheaterFocusStatusSample | null,
  focusNodeId: string | null,
  currentStatus: GraphWorkflowNodeStatus | undefined,
): boolean {
  if (conversationNodeId !== null) {
    return false;
  }
  return shouldReleaseFocusToFollow(prev, focusNodeId, currentStatus);
}

/**
 * After a reveal is consumed, whether Theater should jump to the producing act.
 * Skip when a session is open or the stage is already on that act (avoids flash).
 */
export function shouldStealFocusForArtifactReveal(options: {
  conversationNodeId: string | null;
  stagePrimaryId: string | null;
  artifactNodeId: string;
}): boolean {
  if (options.conversationNodeId !== null) {
    return false;
  }
  return options.stagePrimaryId !== options.artifactNodeId;
}

/** @deprecated Prefer resolveTheaterFocus —kept for older call sites. */
export function resolveFocusNodeId(
  run: GraphWorkflowRun,
  preferredNodeId: string | null,
): string | null {
  return resolveTheaterFocus(run, preferredNodeId).primaryId;
}
