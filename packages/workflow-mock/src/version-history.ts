import type { Edge, Node, ReactFlowJsonObject } from "@xyflow/react";
import type { DemoWorkflow } from "./fixtures";
import type { WorkflowNodeData } from "./node-data";
import type { WorkflowAnnotationNode } from "./annotation-data";

export type MockWorkflowVersionGraph = ReactFlowJsonObject<
  Node<WorkflowNodeData, "workflow">,
  Edge
> & {
  annotations?: WorkflowAnnotationNode[];
};

/** Represents one immutable, published workflow graph used by the version-history UI. */
export interface MockWorkflowVersion {
  /** Snapshot identifier used by rollback; absent on pre-persistence mock fixtures. */
  id?: string;
  version: string;
  createdAt: string;
  graph: MockWorkflowVersionGraph;
}

/** Groups published mock snapshots by their workflow identity. */
export type MockWorkflowVersions = Record<string, MockWorkflowVersion[]>;

/** Creates deterministic published snapshots while the real workflow-version API is unavailable. */
export function createMockWorkflowVersions(
  workflows: DemoWorkflow[],
): MockWorkflowVersions {
  return Object.fromEntries(
    workflows.map((workflow) => [
      workflow.id,
      [
        {
          version: "2026-08-03T10:00:00.000",
          createdAt: "2026-08-03 10:00",
          graph: graphSnapshot(workflow),
        },
        {
          version: "2026-08-01T09:30:00.000",
          createdAt: "2026-08-01 09:30",
          graph: previousGraphSnapshot(workflow),
        },
      ],
    ]),
  );
}

/** Copies the editable graph boundary without including mutable workflow metadata. */
function graphSnapshot(workflow: DemoWorkflow): MockWorkflowVersionGraph {
  return structuredClone({
    nodes: workflow.nodes,
    edges: workflow.edges,
    viewport: workflow.viewport,
    annotations: workflow.annotations ?? [],
  });
}

/** Creates a visibly older but still connected graph for the version-history restore demonstration. */
function previousGraphSnapshot(
  workflow: DemoWorkflow,
): MockWorkflowVersionGraph {
  const graph = graphSnapshot(workflow);
  const removableNode = [...graph.nodes]
    .reverse()
    .find((node) => node.data.kind !== "start");
  if (removableNode === undefined) {
    return graph;
  }
  return {
    ...graph,
    nodes: graph.nodes.filter((node) => node.id !== removableNode.id),
    edges: graph.edges.filter(
      (edge) =>
        edge.source !== removableNode.id && edge.target !== removableNode.id,
    ),
  };
}
