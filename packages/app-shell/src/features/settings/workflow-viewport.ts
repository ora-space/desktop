import type { WorkflowPosition } from "@ora/workflow-mock";

export const MIN_WORKFLOW_ZOOM = 0.4;
export const MAX_WORKFLOW_ZOOM = 1.8;
export const DEFAULT_WORKFLOW_ZOOM = 1;
export const DEFAULT_WORKFLOW_PAN: WorkflowPosition = { x: 32, y: 32 };

export interface WorkflowViewport {
  zoom: number;
  pan: WorkflowPosition;
}

/** Keeps wheel and toolbar zoom within a range where nodes remain operable. */
export function clampWorkflowZoom(zoom: number): number {
  return Math.min(MAX_WORKFLOW_ZOOM, Math.max(MIN_WORKFLOW_ZOOM, zoom));
}

/**
 * Repositions the board while zooming so the graph point beneath the cursor
 * stays fixed, matching the spatial behavior users expect from image viewers.
 */
export function zoomWorkflowAtPoint(
  viewport: WorkflowViewport,
  nextZoom: number,
  cursor: WorkflowPosition,
): WorkflowViewport {
  const zoom = clampWorkflowZoom(nextZoom);
  const worldPoint = {
    x: (cursor.x - viewport.pan.x) / viewport.zoom,
    y: (cursor.y - viewport.pan.y) / viewport.zoom,
  };
  return {
    zoom,
    pan: {
      x: cursor.x - worldPoint.x * zoom,
      y: cursor.y - worldPoint.y * zoom,
    },
  };
}

/** Converts wheel delta into smooth multiplicative scaling for wheels and trackpads. */
export function workflowWheelZoom(currentZoom: number, deltaY: number): number {
  return clampWorkflowZoom(currentZoom * Math.exp(-deltaY * 0.0015));
}
