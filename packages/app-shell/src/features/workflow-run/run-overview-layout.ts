import { workflowContainerNodes } from "@ora/workflow-runtime";
import type {
  GraphWorkflowNodeState,
  WorkflowDefinition,
} from "@ora/workflow-runtime";
import type { Node } from "@xyflow/react";
import type { RunOverviewNodeData } from "./run-overview-node";

/** Builds the read-only Overview nodes with Loop containment and live status data. */
export function createRunOverviewNodes(
  snapshot: WorkflowDefinition,
  nodeStates: Record<string, GraphWorkflowNodeState>,
): Node<RunOverviewNodeData, "workflow">[] {
  return workflowContainerNodes(snapshot.nodes).map((node) => ({
    ...node,
    type: "workflow",
    selectable: true,
    draggable: false,
    connectable: false,
    deletable: false,
    extent: node.parentId === undefined ? undefined : ("parent" as const),
    expandParent: false,
    zIndex: node.data.kind === "loop" ? 0 : 1,
    data: {
      ...node.data,
      runStatus: nodeStates[node.id]?.status ?? "idle",
    },
  }));
}
