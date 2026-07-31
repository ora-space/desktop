import type { WorkflowPosition } from "@ora/workflow-mock";

export const MIN_WORKFLOW_ZOOM = 0.4;
export const MAX_WORKFLOW_ZOOM = 1.8;
export const DEFAULT_WORKFLOW_ZOOM = 1;
export const DEFAULT_WORKFLOW_PAN: WorkflowPosition = { x: 32, y: 32 };

/** Keeps wheel and toolbar zoom within a range where nodes remain operable. */
export function clampWorkflowZoom(zoom: number): number {
  return Math.min(MAX_WORKFLOW_ZOOM, Math.max(MIN_WORKFLOW_ZOOM, zoom));
}
