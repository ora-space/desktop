import type { GraphWorkflowNodeStatus, GraphWorkflowRunStatus } from "./types";

/** Backend run lifecycle states that the adapter projection maps onto. */
export type BackendRunStatus = "pending" | "running" | "succeeded" | "failed" | "cancelled";

/** Backend node-run lifecycle states that the adapter projection maps onto. */
export type BackendNodeStatus = "pending" | "running" | "succeeded" | "failed" | "cancelled";

/**
 * Projects a backend run status onto the frontend display model.
 *
 * The backend models a HITL pause as `Pending` with a non-empty `current_nodes` set, so the
 * `awaiting_input` display state is derived here instead of being a wire status. `partial_failed`
 * has no backend equivalent (a node failure fails the whole run) and is never produced.
 */
export function projectRunStatus(
  status: BackendRunStatus,
  currentNodes: readonly string[],
): GraphWorkflowRunStatus {
  if (status === "pending") {
    return currentNodes.length > 0 ? "awaiting_input" : "pending";
  }
  return status;
}

/**
 * Projects one backend node-run onto the frontend display model.
 *
 * A graph node with no node-run row is `idle`; a `Pending` node-run is a paused HITL gate
 * (`awaiting_input`). `skipped` has no backend equivalent — condition branches that were not
 * taken simply have no node-run rows and project as `idle`.
 */
export function projectNodeStatus(nodeRun: { status: BackendNodeStatus } | null): GraphWorkflowNodeStatus {
  if (nodeRun === null) {
    return "idle";
  }
  if (nodeRun.status === "pending") {
    return "awaiting_input";
  }
  return nodeRun.status;
}
