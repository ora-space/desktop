import { describe, expect, it } from "vitest";
import {
  isoToWorkflowTimestamp,
  parseWorkflowGraph,
  serializeWorkflowGraph,
  workflowTimestampToIso,
} from "./graph-codec";
import type { WorkflowDefinitionEdge, WorkflowDefinitionNode } from "./types";

const node: WorkflowDefinitionNode = {
  id: "start",
  type: "workflow",
  position: { x: 12, y: 34 },
  data: { kind: "start", title: "Start", description: "Receives input" },
};

const edge: WorkflowDefinitionEdge = {
  id: "e1",
  source: "start",
  target: "agent-1",
  type: "workflow",
};

describe("graph envelope codec", () => {
  it("round-trips nodes, edges, viewport, and description", () => {
    const graph = serializeWorkflowGraph({
      nodes: [node],
      edges: [edge],
      viewport: { x: 32, y: 64, zoom: 1.5 },
      description: "A review flow",
    });

    expect(parseWorkflowGraph(graph)).toEqual({
      nodes: [node],
      edges: [edge],
      viewport: { x: 32, y: 64, zoom: 1.5 },
      description: "A review flow",
    });
  });

  it("omits the description key when absent", () => {
    const graph = serializeWorkflowGraph({
      nodes: [node],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
    });

    expect(JSON.parse(graph)).not.toHaveProperty("description");
    expect(parseWorkflowGraph(graph)).not.toHaveProperty("description");
  });

  it("tolerates partial envelopes with missing arrays", () => {
    const graph = JSON.stringify({ viewport: { x: 5, y: 6, zoom: 1 } });

    expect(parseWorkflowGraph(graph)).toEqual({
      nodes: [],
      edges: [],
      viewport: { x: 5, y: 6, zoom: 1 },
    });
  });

  it("falls back to defaults for invalid JSON and non-object envelopes", () => {
    expect(parseWorkflowGraph("not json")).toEqual({
      nodes: [],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
    });
    expect(parseWorkflowGraph("[1,2]")).toEqual({
      nodes: [],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
    });
  });

  it("preserves unknown fields through the round-trip", () => {
    const graph = JSON.stringify({
      nodes: [node],
      edges: [edge],
      viewport: { x: 0, y: 0, zoom: 1 },
      customMetadata: { owner: "rhythm" },
    });

    expect(parseWorkflowGraph(graph)).toMatchObject({
      customMetadata: { owner: "rhythm" },
    });
  });
});

describe("workflow timestamp projection", () => {
  it("converts epoch millis to an ISO string", () => {
    expect(workflowTimestampToIso(0)).toBe("1970-01-01T00:00:00.000Z");
  });

  it("round-trips an ISO string through the epoch-millis projection", () => {
    const iso = "2026-08-05T08:00:00.000Z";
    expect(workflowTimestampToIso(isoToWorkflowTimestamp(iso))).toBe(iso);
  });
});
