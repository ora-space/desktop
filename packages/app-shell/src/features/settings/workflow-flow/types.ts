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
  MockWorkflowVersion,
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
  onAddNode: (kind: WorkflowNodeKind, position: XYPosition) => void;
  onConnect: OnConnect;
  onReconnect: OnReconnect<Edge>;
  libraryCollapsed: boolean;
  inspectorCollapsed: boolean;
  inspectorAvailable: boolean;
  onExpandLibrary: () => void;
  onExpandInspector: () => void;
  versionHistory: MockWorkflowVersion[];
  previewedVersion: MockWorkflowVersion | null;
  /** Formatted last-edit time of the draft (workflow_snapshots.updated_at). */
  draftUpdatedAt?: string;
  onPreviewVersion: (version: MockWorkflowVersion | null) => void;
  onRestoreVersion: (version: MockWorkflowVersion) => void;
  onDeleteVersion: (version: MockWorkflowVersion) => void;
  readOnly: boolean;
}
