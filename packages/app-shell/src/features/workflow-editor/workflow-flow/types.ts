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
  WorkflowAnnotationData,
  WorkflowAnnotationNode,
  WorkflowNodeData,
  WorkflowNodeKind,
} from "@ora/workflow-mock";

export type WorkflowCanvasNode =
  Node<WorkflowNodeData, "workflow"> | WorkflowAnnotationNode;

/** Defines the React Flow element boundary consumed by the workflow canvas. */
export interface WorkflowCanvasProps {
  capabilities: WorkflowCapabilities;
  nodes: Node<WorkflowNodeData, "workflow">[];
  annotations: WorkflowAnnotationNode[];
  edges: Edge[];
  initialViewport: Viewport;
  onNodesChange: OnNodesChange<WorkflowCanvasNode>;
  onEdgesChange: OnEdgesChange<Edge>;
  onAddNode: (kind: WorkflowNodeKind, position: XYPosition) => void;
  onAddAnnotation: (position: XYPosition) => void;
  onUpdateAnnotation: (
    id: string,
    data: Partial<WorkflowAnnotationData>,
  ) => void;
  onOrganize: () => void;
  onConnect: OnConnect;
  onReconnect: OnReconnect<Edge>;
  inspectorCollapsed: boolean;
  inspectorAvailable: boolean;
  onExpandInspector: () => void;
  versionHistory: MockWorkflowVersion[];
  previewedVersion: MockWorkflowVersion | null;
  /** Version string of the workflow's currently active published snapshot, if any. */
  activeVersion: string | null;
  /** Formatted last-edit time of the draft (workflow_snapshots.updated_at). */
  draftUpdatedAt?: string;
  onPreviewVersion: (version: MockWorkflowVersion | null) => void;
  onActivateVersion: (version: MockWorkflowVersion) => void;
  /** Opens the same publish flow as the header, freezing the current draft. */
  onPublishDraft: () => void;
  onDeleteVersion: (version: MockWorkflowVersion) => void;
  readOnly: boolean;
}
