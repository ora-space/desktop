import { describe, expect, it } from "vitest";
import type { Node } from "@xyflow/react";
import {
  applyIterationContainment,
  type ContainmentWorkflow,
} from "./workflow-iteration-containment";
import type { WorkflowNodeData } from "@ora/workflow-mock";

function workflowNode(
  id: string,
  kind: WorkflowNodeData["kind"],
  position: { x: number; y: number },
  parentId?: string,
): Node<WorkflowNodeData, "workflow"> {
  return {
    id,
    type: "workflow",
    position,
    data: {
      kind,
      title: id,
      description: "",
    },
    ...(parentId === undefined ? {} : { parentId }),
  };
}

function workflowOf(
  nodes: Node<WorkflowNodeData, "workflow">[],
): ContainmentWorkflow {
  return { nodes };
}

describe("applyIterationContainment", () => {
  it("adopts a node dropped inside an iteration frame", () => {
    const workflow = workflowOf([
      workflowNode("iter", "iteration", { x: 0, y: 0 }),
      workflowNode("fix", "agent", { x: 100, y: 200 }),
    ]);
    const next = applyIterationContainment(workflow, [
      workflowNode("fix", "agent", { x: 100, y: 200 }),
    ]);
    expect(next).not.toBe(workflow);
    const fix = next.nodes.find((node) => node.id === "fix");
    expect(fix?.parentId).toBe("iter");
    expect(fix?.extent).toBe("parent");
    expect(fix?.position).toEqual({ x: 100, y: 200 });
  });

  it("releases a member dragged outside its frame back to the outer canvas", () => {
    // React Flow applied the drag to the draft before drag-stop: the member's stored
    // position is its new frame-relative coordinate far outside the frame bounds.
    const workflow = workflowOf([
      workflowNode("iter", "iteration", { x: 0, y: 0 }),
      workflowNode("fix", "agent", { x: 900, y: 900 }, "iter"),
    ]);
    const next = applyIterationContainment(workflow, [workflow.nodes[1]!]);
    expect(next).not.toBe(workflow);
    const fix = next.nodes.find((node) => node.id === "fix");
    expect(fix?.parentId).toBeUndefined();
    expect(fix?.extent).toBeUndefined();
    expect(fix?.position).toEqual({ x: 900, y: 900 });
  });

  it("keeps a member dragged within the same frame", () => {
    const workflow = workflowOf([
      workflowNode("iter", "iteration", { x: 0, y: 0 }),
      workflowNode("fix", "agent", { x: 60, y: 180 }, "iter"),
    ]);
    const next = applyIterationContainment(workflow, [workflow.nodes[1]!]);
    expect(next).toBe(workflow);
  });

  it("never adopts output or nested iteration nodes", () => {
    const workflow = workflowOf([
      workflowNode("iter", "iteration", { x: 0, y: 0 }),
      workflowNode("out", "output", { x: 100, y: 200 }),
      workflowNode("inner", "iteration", { x: 100, y: 240 }),
    ]);
    const next = applyIterationContainment(workflow, [
      workflowNode("out", "output", { x: 100, y: 200 }),
      workflowNode("inner", "iteration", { x: 100, y: 240 }),
    ]);
    expect(next).toBe(workflow);
  });

  it("uses the collapsed frame height so collapsed frames do not swallow drops", () => {
    const workflow = workflowOf([
      {
        ...workflowNode("iter", "iteration", { x: 0, y: 0 }),
        data: {
          kind: "iteration" as const,
          title: "iter",
          description: "",
          collapsed: true,
        },
      },
      workflowNode("fix", "agent", { x: 100, y: 160 }),
    ]);
    // y=160 sits inside the expanded frame (340 tall) but below the collapsed one (112).
    const next = applyIterationContainment(workflow, [
      workflowNode("fix", "agent", { x: 100, y: 160 }),
    ]);
    expect(next).toBe(workflow);
  });

  it("returns the same workflow when no iterations exist", () => {
    const workflow = workflowOf([
      workflowNode("a", "agent", { x: 0, y: 0 }),
      workflowNode("b", "agent", { x: 300, y: 0 }),
    ]);
    const next = applyIterationContainment(workflow, [workflow.nodes[1]!]);
    expect(next).toBe(workflow);
  });
});
