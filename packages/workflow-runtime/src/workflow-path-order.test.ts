import { describe, expect, it } from "vitest";
import { normalizeWorkflowDefinition } from "./definition";
import { workflowPathNodes, workflowPathOrder } from "./workflow-path-order";
import type { WorkflowDefinitionInput } from "./definition";

/** Builds a tiny definition; node array order is intentionally not path order. */
function definitionFrom(input: {
  nodes: Array<{
    id: string;
    x: number;
    y: number;
    kind?: "start" | "agent" | "output";
  }>;
  edges: Array<{ source: string; target: string }>;
}): ReturnType<typeof normalizeWorkflowDefinition> {
  const payload: WorkflowDefinitionInput = {
    id: "path-order-fixture",
    name: "Path order",
    description: "",
    updatedAt: "2026-08-08T12:00:00+08:00",
    viewport: { x: 0, y: 0, zoom: 1 },
    nodes: input.nodes.map((node) => ({
      id: node.id,
      position: { x: node.x, y: node.y },
      data: {
        kind: node.kind ?? (node.id === "start" ? "start" : "agent"),
        title: node.id,
        description: "",
        instruction: "",
      },
    })),
    edges: input.edges.map((edge, index) => ({
      id: `e-${index}`,
      source: edge.source,
      target: edge.target,
      type: "workflow",
    })),
  };
  return normalizeWorkflowDefinition(payload);
}

describe("workflowPathOrder", () => {
  it("follows topology even when the snapshot array is reversed", () => {
    const definition = definitionFrom({
      // Array order C → B → A; dependency A → B → C; left-to-right layout.
      nodes: [
        { id: "c", x: 600, y: 100 },
        { id: "b", x: 300, y: 100 },
        { id: "a", x: 0, y: 100, kind: "start" },
      ],
      edges: [
        { source: "a", target: "b" },
        { source: "b", target: "c" },
      ],
    });

    expect(workflowPathOrder(definition)).toEqual(["a", "b", "c"]);
    expect(definition.nodes.map((node) => node.id)).toEqual(["c", "b", "a"]);
  });

  it("tie-breaks concurrent ready nodes by x then y then id", () => {
    const definition = definitionFrom({
      // Snapshot lists docs before security; canvas has security above docs.
      nodes: [
        { id: "start", x: 0, y: 200, kind: "start" },
        { id: "docs", x: 300, y: 320, kind: "agent" },
        { id: "security", x: 300, y: 80, kind: "agent" },
        { id: "merge", x: 600, y: 200 },
      ],
      edges: [
        { source: "start", target: "docs" },
        { source: "start", target: "security" },
        { source: "docs", target: "merge" },
        { source: "security", target: "merge" },
      ],
    });

    expect(workflowPathOrder(definition)).toEqual([
      "start",
      "security",
      "docs",
      "merge",
    ]);
  });

  it("never lets position override a predecessor edge", () => {
    const definition = definitionFrom({
      // B is drawn left of A but depends on A.
      nodes: [
        { id: "b", x: 0, y: 100 },
        { id: "a", x: 400, y: 100, kind: "start" },
      ],
      edges: [{ source: "a", target: "b" }],
    });

    expect(workflowPathOrder(definition)).toEqual(["a", "b"]);
  });

  it("resolves path nodes in the same order", () => {
    const definition = definitionFrom({
      nodes: [
        { id: "end", x: 200, y: 0 },
        { id: "start", x: 0, y: 0, kind: "start" },
      ],
      edges: [{ source: "start", target: "end" }],
    });

    expect(workflowPathNodes(definition).map((node) => node.id)).toEqual([
      "start",
      "end",
    ]);
  });
});
