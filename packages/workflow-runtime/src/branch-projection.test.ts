import { describe, expect, it } from "vitest";
import { computeInactiveNodes } from "./branch-projection";
import type { GraphWorkflowNodeStatus, WorkflowDefinition } from "./types";

function definition(
  nodes: Array<{ id: string; kind: string }>,
  edges: WorkflowDefinition["edges"],
): WorkflowDefinition {
  return {
    id: "wf-1",
    name: "branching",
    description: "",
    updatedAt: "",
    viewport: { x: 0, y: 0, zoom: 1 },
    nodes: nodes.map((node) => ({
      id: node.id,
      type: "workflow",
      position: { x: 0, y: 0 },
      data: {
        kind: node.kind,
        title: "",
        description: "",
      } as WorkflowDefinition["nodes"][number]["data"],
    })),
    edges,
  };
}

/** A branch-and-remerge shape: start → c → {ok, no}. */
function branchDefinition(
  edges: WorkflowDefinition["edges"],
): WorkflowDefinition {
  return definition(
    [
      { id: "start", kind: "start" },
      { id: "c", kind: "condition" },
      { id: "ok", kind: "agent" },
      { id: "no", kind: "agent" },
    ],
    edges,
  );
}

function statuses(map: Record<string, GraphWorkflowNodeStatus>) {
  return map;
}

describe("computeInactiveNodes", () => {
  it("activates only the selected branch and deactivates its downstream nodes", () => {
    const def = branchDefinition([
      { id: "e1", source: "start", target: "c" },
      { id: "e2", source: "c", sourceHandle: "approved", target: "ok" },
      { id: "e3", source: "c", sourceHandle: "else", target: "no" },
    ]);
    const inactive = computeInactiveNodes(
      def,
      statuses({ start: "succeeded", c: "succeeded" }),
      { c: "approved" },
    );
    expect(inactive.has("ok")).toBe(false);
    expect(inactive.has("no")).toBe(true);
  });

  it("marks a fan-in node inactive when every incoming branch is lost", () => {
    const def = definition(
      [
        { id: "c", kind: "condition" },
        { id: "left", kind: "agent" },
        { id: "right", kind: "agent" },
        { id: "merge", kind: "agent" },
      ],
      [
        { id: "e1", source: "c", sourceHandle: "approved", target: "left" },
        { id: "e2", source: "c", sourceHandle: "else", target: "right" },
        { id: "e3", source: "left", target: "merge" },
        { id: "e4", source: "right", target: "merge" },
      ],
    );
    // The condition picks a branch no edge feeds, so both branch agents and the merge are lost.
    const inactive = computeInactiveNodes(def, statuses({ c: "succeeded" }), {
      c: "case-x",
    });
    expect(inactive.has("left")).toBe(true);
    expect(inactive.has("right")).toBe(true);
    expect(inactive.has("merge")).toBe(true);
  });

  it("treats an unlabeled condition edge as the else branch", () => {
    const def = branchDefinition([{ id: "e1", source: "c", target: "no" }]);
    const inactive = computeInactiveNodes(def, statuses({ c: "succeeded" }), {
      c: "else",
    });
    expect(inactive.has("no")).toBe(false);
  });

  it("leaves a downstream node active in a graph without conditions", () => {
    const def = branchDefinition([{ id: "e1", source: "start", target: "c" }]);
    const inactive = computeInactiveNodes(
      def,
      statuses({ start: "succeeded" }),
      {},
    );
    expect(inactive.has("c")).toBe(false);
  });

  it("keeps a node on a running branch active", () => {
    const def = branchDefinition([
      { id: "e1", source: "c", sourceHandle: "approved", target: "ok" },
      { id: "e2", source: "c", sourceHandle: "else", target: "no" },
    ]);
    const inactive = computeInactiveNodes(
      def,
      statuses({ c: "succeeded", ok: "running" }),
      { c: "approved" },
    );
    expect(inactive.has("ok")).toBe(false);
    expect(inactive.has("no")).toBe(true);
  });
});
