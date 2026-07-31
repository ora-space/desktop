import { describe, expect, it } from "vitest";
import {
  MAX_WORKFLOW_ZOOM,
  MIN_WORKFLOW_ZOOM,
  clampWorkflowZoom,
} from "./viewport";

describe("workflow viewport", () => {
  it("clamps direct zoom to the supported range", () => {
    expect({
      directMinimum: clampWorkflowZoom(-10),
      directMaximum: clampWorkflowZoom(10),
    }).toEqual({
      directMinimum: MIN_WORKFLOW_ZOOM,
      directMaximum: MAX_WORKFLOW_ZOOM,
    });
  });
});
