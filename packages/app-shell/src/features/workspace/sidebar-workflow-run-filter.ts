import type { WorkflowRunStatus } from "@ora/contracts";
import {
  toDisplayRunStatus,
  type GraphWorkflowRunStatus,
} from "@ora/workflow-runtime";
import type { SidebarWorkflowRunStatusFilter } from "../../state/stores/ui-store";
import { runStatusTone } from "../workflow-run/run-status-style";

/** Display statuses offered by the sidebar workflow filter, including All. */
export const SIDEBAR_WORKFLOW_RUN_STATUS_FILTERS = [
  "all",
  "pending",
  "running",
  "awaiting_input",
  "succeeded",
  "failed",
  "cancelled",
] as const satisfies readonly SidebarWorkflowRunStatusFilter[];

/**
 * Status-dot fill for the workflow filter menu. Reuses Theater tones so the
 * menu matches sidebar tree dots after the status-color sync.
 */
export function sidebarRunStatusDotClass(
  status: GraphWorkflowRunStatus,
): string {
  return runStatusTone(status).dot;
}

/** True when a list row should stay visible under the project's status and search filters. */
export function workflowRunMatchesSidebarFilters(
  run: { name: string; version: string; status: WorkflowRunStatus },
  statusFilter: SidebarWorkflowRunStatusFilter,
  searchNeedle: string,
): boolean {
  if (
    statusFilter !== "all" &&
    toDisplayRunStatus(run.status) !== statusFilter
  ) {
    return false;
  }
  if (searchNeedle.length === 0) return true;
  return (
    run.name.toLowerCase().includes(searchNeedle) ||
    run.version.toLowerCase().includes(searchNeedle)
  );
}
