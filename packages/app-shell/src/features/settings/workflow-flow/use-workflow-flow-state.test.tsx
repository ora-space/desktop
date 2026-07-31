import { act, renderHook } from "@testing-library/react";
import { ReactFlowProvider } from "@xyflow/react";
import { describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { WorkflowEdge, WorkflowNode } from "@ora/workflow-mock";
import { useWorkflowFlowState } from "./use-workflow-flow-state";

const nodes: WorkflowNode[] = [
  {
    id: "start",
    kind: "start",
    title: "Start",
    description: "Entry",
    position: { x: 0, y: 0 },
    config: { instruction: "go" },
  },
  {
    id: "output",
    kind: "output",
    title: "Output",
    description: "Exit",
    position: { x: 200, y: 0 },
    config: { instruction: "done" },
  },
  {
    id: "other",
    kind: "tool",
    title: "Other",
    description: "Alternate",
    position: { x: 200, y: 120 },
    config: { instruction: "check" },
  },
];

const edges: WorkflowEdge[] = [
  { id: "edge-start-output", source: "start", target: "output" },
];

/** Provides the internal React Flow store required by the synchronization hook. */
function wrapper({ children }: { children: ReactNode }) {
  return <ReactFlowProvider>{children}</ReactFlowProvider>;
}

/** Creates stable mutation spies for one hook test. */
function callbacks() {
  return {
    onSelectNode: vi.fn(),
    onMoveNode: vi.fn(),
    onConnect: vi.fn(),
    onReconnectEdge: vi.fn(),
    onDeleteNode: vi.fn(),
    onDeleteEdge: vi.fn(),
  };
}

describe("useWorkflowFlowState", () => {
  it("keeps pointer-frequency movement local and commits only the final position", () => {
    const mutations = callbacks();
    const { result } = renderHook(
      () => useWorkflowFlowState({
        nodes,
        edges,
        selectedNodeId: null,
        ...mutations,
      }),
      { wrapper },
    );

    act(() => {
      result.current.handleNodesChange([
        {
          id: "start",
          type: "position",
          position: { x: 40, y: 60 },
          dragging: true,
        },
      ]);
    });

    expect(result.current.flowNodes[0]?.position).toEqual({ x: 40, y: 60 });
    expect(mutations.onMoveNode).not.toHaveBeenCalled();

    act(() => {
      const movedNode = result.current.flowNodes[0];
      if (movedNode !== undefined) {
        result.current.handleNodeDragStop(
          {} as never,
          movedNode,
          [movedNode],
        );
      }
    });

    expect(mutations.onMoveNode).toHaveBeenCalledExactlyOnceWith(
      "start",
      { x: 40, y: 60 },
    );
  });

  it("validates and commits reconnect gestures through the domain boundary", () => {
    const mutations = callbacks();
    const { result } = renderHook(
      () => useWorkflowFlowState({
        nodes,
        edges,
        selectedNodeId: null,
        ...mutations,
      }),
      { wrapper },
    );

    expect(result.current.isValidConnection({
      source: "start",
      target: "start",
      sourceHandle: null,
      targetHandle: null,
    })).toBe(false);
    expect(result.current.isValidConnection({
      source: "start",
      target: "output",
      sourceHandle: null,
      targetHandle: null,
    })).toBe(false);
    expect(result.current.isValidConnection({
      source: "start",
      target: "other",
      sourceHandle: null,
      targetHandle: null,
    })).toBe(true);

    act(() => {
      result.current.handleReconnect(
        result.current.flowEdges[0]!,
        {
          source: "start",
          target: "other",
          sourceHandle: null,
          targetHandle: null,
        },
      );
    });

    expect(mutations.onReconnectEdge).toHaveBeenCalledExactlyOnceWith(
      "edge-start-output",
      "start",
      "other",
    );
  });
});
