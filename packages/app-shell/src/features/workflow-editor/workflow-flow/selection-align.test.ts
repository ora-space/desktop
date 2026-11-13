import { describe, expect, it } from "vitest";
import type { Node } from "@xyflow/react";
import { alignSelectedNodes } from "./selection-align";

function node(
  id: string,
  position: { x: number; y: number },
  extras: Partial<Node> = {},
): Node {
  return {
    id,
    position,
    data: {},
    selected: true,
    ...extras,
  };
}

describe("alignSelectedNodes", () => {
  it("left-aligns selected roots to the leftmost x", () => {
    const nodes = [
      node("a", { x: 40, y: 10 }),
      node("b", { x: 120, y: 40 }),
      node("c", { x: 200, y: 0 }, { selected: false }),
    ];
    const next = alignSelectedNodes(nodes, "left");
    expect(next?.find((item) => item.id === "a")?.position).toEqual({
      x: 40,
      y: 10,
    });
    expect(next?.find((item) => item.id === "b")?.position).toEqual({
      x: 40,
      y: 40,
    });
    expect(next?.find((item) => item.id === "c")?.position).toEqual({
      x: 200,
      y: 0,
    });
  });

  it("top-aligns selected roots to the topmost y", () => {
    const nodes = [node("a", { x: 10, y: 80 }), node("b", { x: 60, y: 20 })];
    const next = alignSelectedNodes(nodes, "top");
    expect(next?.find((item) => item.id === "a")?.position.y).toBe(20);
    expect(next?.find((item) => item.id === "b")?.position.y).toBe(20);
  });

  it("distributes three nodes with equal gaps and keeps the ends fixed", () => {
    const nodes = [
      node("a", { x: 0, y: 0 }, { width: 100 }),
      node("b", { x: 130, y: 10 }, { width: 100 }),
      node("c", { x: 400, y: 5 }, { width: 100 }),
    ];
    const next = alignSelectedNodes(nodes, "distribute-x");
    expect(next?.find((item) => item.id === "a")?.position.x).toBe(0);
    expect(next?.find((item) => item.id === "c")?.position.x).toBe(400);
    expect(next?.find((item) => item.id === "b")?.position.x).toBe(200);
  });

  it("refuses a mixed parent selection", () => {
    const nodes = [
      node("a", { x: 0, y: 0 }),
      node("b", { x: 40, y: 0 }, { parentId: "frame" }),
    ];
    expect(alignSelectedNodes(nodes, "left")).toBeNull();
  });

  it("does not move a child when its parent is also selected", () => {
    const frame = node("frame", { x: 0, y: 0 });
    const child = node("child", { x: 16, y: 24 }, { parentId: "frame" });
    const peer = node("peer", { x: 80, y: 0 });
    const next = alignSelectedNodes([frame, child, peer], "left");
    expect(next?.find((item) => item.id === "child")?.position).toEqual({
      x: 16,
      y: 24,
    });
    expect(next?.find((item) => item.id === "peer")?.position.x).toBe(0);
  });
});
