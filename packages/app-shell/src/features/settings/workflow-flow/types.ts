import type {
  Edge,
  Node,
  OnConnect,
  OnEdgesChange,
  OnNodesChange,
  OnReconnect,
  Viewport,
  XYPosition,
} from "@xyflow/react";
import type {
  WorkflowCapabilities,
  WorkflowNodeData,
  WorkflowNodeKind,
} from "@ora/workflow-mock";

/** Defines the React Flow element boundary consumed by the workflow canvas. */
export interface WorkflowCanvasProps {
  capabilities: WorkflowCapabilities;
  nodes: Node<WorkflowNodeData, "workflow">[];
  edges: Edge[];
  initialViewport: Viewport;
  onNodesChange: OnNodesChange<Node<WorkflowNodeData, "workflow">>;
  onEdgesChange: OnEdgesChange<Edge>;
  onViewportChange: (viewport: Viewport) => void;
  onAddNode: (kind: WorkflowNodeKind, position: XYPosition) => void;
  onConnect: OnConnect;
  onReconnect: OnReconnect<Edge>;
  libraryCollapsed: boolean;
  inspectorCollapsed: boolean;
  inspectorAvailable: boolean;
  onExpandLibrary: () => void;
  onExpandInspector: () => void;
}
