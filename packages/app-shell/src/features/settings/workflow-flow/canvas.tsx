import {
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useTranslation } from "react-i18next";
import {
  Background,
  BackgroundVariant,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Connection,
  type DefaultEdgeOptions,
  type Edge,
  type FinalConnectionState,
  type HandleType,
  type OnConnectStartParams,
  type Viewport,
} from "@xyflow/react";
import type {
  WorkflowNodeKind,
} from "@ora/workflow-mock";
import { WorkflowNodeCatalog } from "../workflow-node-catalog";
import {
  DEFAULT_WORKFLOW_PAN,
  DEFAULT_WORKFLOW_ZOOM,
  MAX_WORKFLOW_ZOOM,
  MIN_WORKFLOW_ZOOM,
} from "./viewport";
import {
  WORKFLOW_FLOW_EDGE_TYPE,
  WORKFLOW_FLOW_NODE_TYPE,
  WORKFLOW_SNAP_GRID,
  nodePositionAt,
  snapNodePosition,
} from "./adapters";
import { WorkflowFlowCallbacksProvider } from "./callbacks";
import { WorkflowConnectionLine } from "./connection-line";
import { WorkflowCanvasControls } from "./controls";
import { WorkflowFlowEdgeView } from "./edge";
import { WorkflowFlowNodeView } from "./node";
import { WorkflowFlowOverview } from "./overview";
import type { ClientPosition, WorkflowCanvasProps } from "./types";
import { useWorkflowFlowState } from "./use-workflow-flow-state";
import "@xyflow/react/dist/style.css";
import "./workflow-flow.css";

const nodeTypes = {
  [WORKFLOW_FLOW_NODE_TYPE]: WorkflowFlowNodeView,
};

const edgeTypes = {
  [WORKFLOW_FLOW_EDGE_TYPE]: WorkflowFlowEdgeView,
};

const DEFAULT_VIEWPORT: Viewport = {
  x: DEFAULT_WORKFLOW_PAN.x,
  y: DEFAULT_WORKFLOW_PAN.y,
  zoom: DEFAULT_WORKFLOW_ZOOM,
};
const DEFAULT_EDGE_OPTIONS = {
  type: WORKFLOW_FLOW_EDGE_TYPE,
} satisfies DefaultEdgeOptions;
const CONNECTION_LINE_STYLE = {
  stroke: "var(--ring)",
  strokeWidth: 2,
  strokeDasharray: "5 4",
} satisfies CSSProperties;

type ConnectionDraft =
  | {
      kind: "new";
      source: string;
    }
  | {
      kind: "reconnect";
      edgeId: string;
      endpoint: HandleType;
      source: string;
      target: string;
    };

/** Finds the workflow card under a pointer so the whole card remains a forgiving drop zone. */
function workflowNodeAtClientPoint(clientX: number, clientY: number): string | null {
  const element = document.elementFromPoint(clientX, clientY);
  if (!(element instanceof Element)) {
    return null;
  }
  return element
    .closest<HTMLElement>("[data-workflow-node-id]")
    ?.dataset.workflowNodeId ?? null;
}

/** Normalizes mouse and touch releases for whole-card connection fallback. */
function connectionEndClientPoint(
  event: MouseEvent | TouchEvent,
): { clientX: number; clientY: number } | null {
  if ("changedTouches" in event) {
    const touch = event.changedTouches.item(0);
    return touch === null
      ? null
      : { clientX: touch.clientX, clientY: touch.clientY };
  }
  return { clientX: event.clientX, clientY: event.clientY };
}

/** Resolves the directed pair represented by a new or reconnect drag. */
function connectionForCandidate(
  draft: ConnectionDraft,
  candidateNodeId: string,
): Connection {
  if (draft.kind === "new") {
    return {
      source: draft.source,
      target: candidateNodeId,
      sourceHandle: null,
      targetHandle: null,
    };
  }
  return {
    source: draft.endpoint === "source" ? candidateNodeId : draft.source,
    target: draft.endpoint === "target" ? candidateNodeId : draft.target,
    sourceHandle: null,
    targetHandle: null,
  };
}

/** Wraps the flow in a provider so catalog drop can convert screen coordinates. */
export function WorkflowCanvas(props: WorkflowCanvasProps) {
  return (
    <ReactFlowProvider>
      <WorkflowCanvasInner {...props} />
    </ReactFlowProvider>
  );
}

/** Renders and manipulates the node graph without coupling it to persistence or preview behavior. */
function WorkflowCanvasInner({
  capabilities,
  nodes,
  edges,
  selectedNodeId,
  onSelectNode,
  onMoveNode,
  onAddNode,
  onConnect,
  onReconnectEdge,
  onDeleteNode,
  onDeleteEdge,
  libraryCollapsed,
  inspectorCollapsed,
  inspectorAvailable,
  onExpandLibrary,
  onExpandInspector,
}: WorkflowCanvasProps) {
  const { t } = useTranslation();
  const canvasRef = useRef<HTMLDivElement>(null);
  const connectionDraftRef = useRef<ConnectionDraft | null>(null);
  const [connectionCandidateNodeId, setConnectionCandidateNodeId] =
    useState<string | null>(null);
  const { screenToFlowPosition } = useReactFlow();
  const {
    clearSelection,
    deleteEdge,
    flowCallbacks,
    flowEdges,
    flowNodes,
    handleConnect,
    handleEdgesChange,
    handleNodeDragStop,
    handleNodesChange,
    handleReconnect,
    isValidConnection,
    reconnectingEdgeIdRef,
    selectEdge,
    setSelectedEdgeId,
  } = useWorkflowFlowState({
    nodes,
    edges,
    selectedNodeId,
    onSelectNode,
    onMoveNode,
    onConnect,
    onReconnectEdge,
    onDeleteNode,
    onDeleteEdge,
  });
  const rendererCallbacks = useMemo(
    () => {
      const draft = connectionDraftRef.current;
      return {
        ...flowCallbacks,
        connectionCandidateEndpoint: connectionCandidateNodeId === null
          ? null
          : draft?.kind === "new"
            ? "target" as const
            : draft?.endpoint ?? null,
        connectionCandidateNodeId,
      };
    },
    [connectionCandidateNodeId, flowCallbacks],
  );

  /** Clears connection-only state after React Flow has completed or cancelled a gesture. */
  function finishConnectionGesture(): void {
    connectionDraftRef.current = null;
    reconnectingEdgeIdRef.current = null;
    setConnectionCandidateNodeId(null);
  }

  /** Updates the whole-card candidate highlight without writing graph state on pointer move. */
  function updateConnectionCandidate(
    event: ReactPointerEvent<HTMLDivElement>,
  ): void {
    const draft = connectionDraftRef.current;
    if (draft === null) {
      return;
    }
    const candidate = workflowNodeAtClientPoint(event.clientX, event.clientY);
    const validCandidate = candidate !== null
      && isValidConnection(connectionForCandidate(draft, candidate))
      ? candidate
      : null;
    setConnectionCandidateNodeId((current) =>
      current === validCandidate ? current : validCandidate,
    );
  }

  /** Records a source drag so nearby cards can provide the original forgiving target. */
  function startConnection(params: OnConnectStartParams): void {
    // React Flow also emits the generic connection lifecycle while reconnecting.
    // The reconnect draft must remain authoritative or a moved endpoint becomes
    // an accidental new edge.
    if (
      reconnectingEdgeIdRef.current === null
      && params.nodeId !== null
      && params.handleType === "source"
    ) {
      connectionDraftRef.current = {
        kind: "new",
        source: params.nodeId,
      };
    }
  }

  /** Commits a card drop when React Flow did not hit the card's smaller target handle. */
  function finishNewConnection(
    event: MouseEvent | TouchEvent,
    connectionState: FinalConnectionState,
  ): void {
    const draft = connectionDraftRef.current;
    // A reconnect has its own end callback. Clearing it from this generic
    // callback makes the later reconnect end look like a cancelled gesture.
    if (draft?.kind !== "new") {
      return;
    }
    const point = connectionEndClientPoint(event);
    if (
      connectionState.isValid !== true
      && point !== null
    ) {
      const candidate = workflowNodeAtClientPoint(point.clientX, point.clientY);
      if (candidate !== null) {
        const connection = connectionForCandidate(draft, candidate);
        if (isValidConnection(connection)) {
          handleConnect(connection);
        }
      }
    }
    finishConnectionGesture();
  }

  /** Commits a source or target reconnect when it is released anywhere on a valid card. */
  function finishReconnect(
    event: MouseEvent | TouchEvent,
    edge: Edge,
    _handleType: HandleType,
    connectionState: FinalConnectionState,
  ): void {
    const draft = connectionDraftRef.current;
    const point = connectionEndClientPoint(event);
    if (
      connectionState.isValid !== true
      && draft?.kind === "reconnect"
      && point !== null
    ) {
      const candidate = workflowNodeAtClientPoint(point.clientX, point.clientY);
      if (candidate !== null) {
        const connection = connectionForCandidate(draft, candidate);
        if (isValidConnection(connection)) {
          handleReconnect(edge, connection);
        }
      }
    }
    finishConnectionGesture();
  }

  /** Adds a clicked catalog item to the center of the currently visible canvas. */
  function addNodeAtViewportCenter(kind: WorkflowNodeKind): void {
    const bounds = canvasRef.current?.getBoundingClientRect();
    if (bounds === undefined) {
      onAddNode(kind, nodePositionAt({ x: 0, y: 0 }));
      return;
    }
    const point = screenToFlowPosition(
      {
        x: bounds.left + bounds.width / 2,
        y: bounds.top + bounds.height / 2,
      },
      { snapToGrid: false },
    );
    onAddNode(kind, snapNodePosition(nodePositionAt(point)));
  }

  /** Adds a pointer-dragged catalog node only when it is released over this canvas. */
  function dropNodeAtClientPosition(
    kind: WorkflowNodeKind,
    position: ClientPosition,
  ): void {
    const bounds = canvasRef.current?.getBoundingClientRect();
    if (
      bounds === undefined
      || position.clientX < bounds.left
      || position.clientX > bounds.right
      || position.clientY < bounds.top
      || position.clientY > bounds.bottom
    ) {
      return;
    }
    onAddNode(
      kind,
      snapNodePosition(
        nodePositionAt(
          screenToFlowPosition(
            {
              x: position.clientX,
              y: position.clientY,
            },
            { snapToGrid: false },
          ),
        ),
      ),
    );
  }

  /**
   * Blocks pan starts in the thin horizontal strip where resizable panel
   * handles overlap the canvas so a near-miss resize never becomes a pan.
   */
  function guardPanelResizeEdge(event: ReactPointerEvent<HTMLDivElement>): void {
    const bounds = event.currentTarget.getBoundingClientRect();
    const nearestHorizontalEdge = Math.min(
      event.clientX - bounds.left,
      bounds.right - event.clientX,
    );
    if (bounds.width > 24 && nearestHorizontalEdge <= 12) {
      event.stopPropagation();
    }
  }

  return (
    <div className="relative min-h-0 min-w-0 flex-1">
      <div
        ref={canvasRef}
        className="absolute inset-0 touch-none outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        aria-label={t("settings.workflow.canvas")}
        data-workflow-edge-count={flowEdges.length}
        data-workflow-node-count={flowNodes.length}
        onPointerDownCapture={guardPanelResizeEdge}
        onPointerMoveCapture={updateConnectionCandidate}
      >
        <WorkflowFlowCallbacksProvider value={rendererCallbacks}>
          <ReactFlow
            className="workflow-flow bg-muted/25"
            nodes={flowNodes}
            edges={flowEdges}
            nodeTypes={nodeTypes}
            edgeTypes={edgeTypes}
            defaultViewport={DEFAULT_VIEWPORT}
            minZoom={MIN_WORKFLOW_ZOOM}
            maxZoom={MAX_WORKFLOW_ZOOM}
            proOptions={{ hideAttribution: true }}
            nodesFocusable
            edgesFocusable
            edgesReconnectable
            reconnectRadius={28}
            connectionRadius={24}
            deleteKeyCode={["Backspace", "Delete"]}
            multiSelectionKeyCode={null}
            snapGrid={WORKFLOW_SNAP_GRID}
            snapToGrid
            panOnScroll={false}
            zoomOnScroll
            zoomOnPinch
            panOnDrag
            selectNodesOnDrag={false}
            isValidConnection={isValidConnection}
            onNodesChange={handleNodesChange}
            onNodeDragStop={handleNodeDragStop}
            onEdgesChange={handleEdgesChange}
            onConnectStart={(_event, params) => {
              startConnection(params);
            }}
            onConnect={handleConnect}
            onConnectEnd={finishNewConnection}
            onReconnectStart={(_event, edge, handleType) => {
              reconnectingEdgeIdRef.current = edge.id;
              connectionDraftRef.current = {
                kind: "reconnect",
                edgeId: edge.id,
                // React Flow reports the fixed opposite handle here: dragging
                // the visible source endpoint therefore reports "target".
                endpoint: handleType === "target" ? "source" : "target",
                source: edge.source,
                target: edge.target,
              };
            }}
            onReconnect={handleReconnect}
            onReconnectEnd={finishReconnect}
            onNodeClick={(_event, node) => {
              setSelectedEdgeId(null);
              onSelectNode(node.id);
            }}
            onEdgeClick={(_event, edge) => {
              selectEdge(edge.id);
            }}
            onPaneClick={clearSelection}
            onEdgeDoubleClick={(_event, edge) => {
              deleteEdge(edge.id);
            }}
            connectionLineComponent={WorkflowConnectionLine}
            elevateEdgesOnSelect
            defaultEdgeOptions={DEFAULT_EDGE_OPTIONS}
            connectionLineStyle={CONNECTION_LINE_STYLE}
          >
            <Background
              id="workflow-dots"
              variant={BackgroundVariant.Dots}
              gap={20}
              size={1}
              color="color-mix(in oklch, var(--foreground) 18%, transparent)"
            />
            <WorkflowFlowOverview nodeCount={flowNodes.length} />
          </ReactFlow>
        </WorkflowFlowCallbacksProvider>

        <WorkflowCanvasControls
          defaultViewport={DEFAULT_VIEWPORT}
          libraryCollapsed={libraryCollapsed}
          inspectorCollapsed={inspectorCollapsed}
          inspectorAvailable={inspectorAvailable}
          onExpandLibrary={onExpandLibrary}
          onExpandInspector={onExpandInspector}
        />
      </div>

      <div
        data-workflow-controls
        className="absolute bottom-3 left-1/2 z-30 w-fit max-w-[calc(100%_-_12rem)] -translate-x-1/2"
      >
        <WorkflowNodeCatalog
          capabilities={capabilities}
          onAdd={addNodeAtViewportCenter}
          onDrop={dropNodeAtClientPosition}
        />
      </div>
    </div>
  );
}
