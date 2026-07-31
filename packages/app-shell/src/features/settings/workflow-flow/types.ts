import type {
  WorkflowCapabilities,
  WorkflowEdge,
  WorkflowNode,
  WorkflowNodeKind,
  WorkflowPosition,
} from "@ora/workflow-mock";

/** Defines the domain boundary consumed by the React Flow workflow editor. */
export interface WorkflowCanvasProps {
  capabilities: WorkflowCapabilities;
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string | null) => void;
  onMoveNode: (nodeId: string, position: WorkflowPosition) => void;
  onAddNode: (kind: WorkflowNodeKind, position: WorkflowPosition) => void;
  onConnect: (source: string, target: string) => void;
  onReconnectEdge: (edgeId: string, source: string, target: string) => void;
  onDeleteNode: (nodeId: string) => void;
  onDeleteEdge: (edgeId: string) => void;
  libraryCollapsed: boolean;
  inspectorCollapsed: boolean;
  inspectorAvailable: boolean;
  onExpandLibrary: () => void;
  onExpandInspector: () => void;
}

export interface ClientPosition {
  clientX: number;
  clientY: number;
}
