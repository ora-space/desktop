import { describe, expect, it } from "vitest";
import {
  MAX_WORKFLOW_ZOOM,
  MIN_WORKFLOW_ZOOM,
  clampWorkflowZoom,
  workflowWheelZoom,
  zoomWorkflowAtPoint,
} from "./workflow-viewport";

describe("workflow viewport", () => {
  it("keeps the graph point beneath the cursor fixed while zooming", () => {
    const cursor = { x: 320, y: 180 };
    const viewport = {
      zoom: 1,
      pan: { x: 20, y: 30 },
    };
    const before = {
      x: (cursor.x - viewport.pan.x) / viewport.zoom,
      y: (cursor.y - viewport.pan.y) / viewport.zoom,
    };

    const zoomed = zoomWorkflowAtPoint(viewport, 1.5, cursor);
    const after = {
      x: (cursor.x - zoomed.pan.x) / zoomed.zoom,
      y: (cursor.y - zoomed.pan.y) / zoomed.zoom,
    };

    expect(after).toEqual(before);
  });

  it("clamps direct and wheel zoom to the supported range", () => {
    expect({
      directMinimum: clampWorkflowZoom(-10),
      directMaximum: clampWorkflowZoom(10),
      wheelMinimum: workflowWheelZoom(1, 100_000),
      wheelMaximum: workflowWheelZoom(1, -100_000),
    }).toEqual({
      directMinimum: MIN_WORKFLOW_ZOOM,
      directMaximum: MAX_WORKFLOW_ZOOM,
      wheelMinimum: MIN_WORKFLOW_ZOOM,
      wheelMaximum: MAX_WORKFLOW_ZOOM,
    });
  });
});
