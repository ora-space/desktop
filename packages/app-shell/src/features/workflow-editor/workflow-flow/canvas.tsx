import {
  useEffect,
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
  MarkerType,
  ReactFlow,
  useReactFlow,
  type Connection,
  type DefaultEdgeOptions,
  type Edge,
  type FinalConnectionState,
  type HandleType,
  type OnConnectStartParams,
  type Viewport,
  type XYPosition,
} from "@xyflow/react";
import type { WorkflowNodeKind } from "@ora/workflow-mock";
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
} from "./layout";
import { WorkflowConnectionStateProvider } from "./connection-state";
import { WorkflowConnectionLine } from "./connection-line";
import {
  WorkflowCanvasControls,
  WorkflowCanvasInspectorRestore,
} from "./controls";
import { WorkflowFlowEdgeView } from "./edge";
import { WorkflowFlowNodeView } from "./node";
import { WorkflowFlowOverview } from "./overview";
import {
  WorkflowAnnotationActionsProvider,
  WorkflowAnnotationView,
} from "./annotation";
import { WorkflowCanvasTools, type CanvasInteractionMode } from "./tools";
import type { WorkflowCanvasNode, WorkflowCanvasProps } from "./types";
import { WorkflowVersionHistory } from "./version-history";
import "@xyflow/react/dist/style.css";
import "./workflow-flow.css";

const nodeTypes = {
  [WORKFLOW_FLOW_NODE_TYPE]: WorkflowFlowNodeView,
  annotation: WorkflowAnnotationView,
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
  reconnectable: true,
  ariaRole: "button",
  markerEnd: {
    type: MarkerType.ArrowClosed,
    width: 28,
    height: 28,
    markerUnits: "userSpaceOnUse",
    color: "color-mix(in oklch, var(--foreground) 64%, transparent)",
  },
} satisfies DefaultEdgeOptions;
const CONNECTION_LINE_STYLE = {
  stroke: "var(--ring)",
  strokeWidth: 2,
  strokeDasharray: "5 4",
} satisfies CSSProperties;
const WORKFLOW_ANNOTATION_WIDTH = 240;
const WORKFLOW_ANNOTATION_HEIGHT = 140;
const WORKFLOW_ANNOTATION_Z_INDEX = 0;
const WORKFLOW_NODE_Z_INDEX = 1;
const WORKFLOW_SELECTED_NODE_Z_INDEX = 1_000;

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
function workflowNodeAtClientPoint(
  clientX: number,
  clientY: number,
): string | null {
  const element = document.elementFromPoint(clientX, clientY);
  if (!(element instanceof Element)) {
    return null;
  }
  return (
    element.closest<HTMLElement>("[data-workflow-node-id]")?.dataset
      .workflowNodeId ?? null
  );
}

/** Normalizes mouse and touch releases for whole-card connection fallback. */
function connectionEndClientPoint(
  event: MouseEvent | TouchEvent,
): XYPosition | null {
  if ("changedTouches" in event) {
    const touch = event.changedTouches.item(0);
    return touch === null ? null : { x: touch.clientX, y: touch.clientY };
  }
  return { x: event.clientX, y: event.clientY };
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
    <WorkflowAnnotationActionsProvider
      value={{
        readOnly: props.readOnly,
        update: props.onUpdateAnnotation,
        remove: (id) => props.onNodesChange([{ id, type: "remove" }]),
      }}
    >
      <WorkflowCanvasInner {...props} />
    </WorkflowAnnotationActionsProvider>
  );
}

/** Renders and manipulates the node graph without coupling it to persistence or preview behavior. */
function WorkflowCanvasInner({
  capabilities,
  nodes,
  annotations,
  edges,
  initialViewport,
  onNodesChange,
  onEdgesChange,
  onAddNode,
  onAddAnnotation,
  onOrganize,
  onConnect,
  onReconnect,
  inspectorCollapsed,
  inspectorAvailable,
  onExpandInspector,
  versionHistory,
  previewedVersion,
  activeVersion,
  draftUpdatedAt,
  onPreviewVersion,
  onActivateVersion,
  onPublishDraft,
  onDeleteVersion,
  readOnly,
}: WorkflowCanvasProps) {
  const { t } = useTranslation();
  const canvasRef = useRef<HTMLDivElement>(null);
  const [interactionMode, setInteractionMode] =
    useState<CanvasInteractionMode>("pointer");
  const [connectionDraft, setConnectionDraft] =
    useState<ConnectionDraft | null>(null);
  const connectionCandidateFrameRef = useRef<number | null>(null);
  const connectionCandidatePointRef = useRef<XYPosition | null>(null);
  const connectionCandidateNodeIdRef = useRef<string | null>(null);
  const [connectionCandidateNodeId, setConnectionCandidateNodeId] = useState<
    string | null
  >(null);
  const { deleteElements, fitView, screenToFlowPosition, setViewport } =
    useReactFlow<WorkflowCanvasNode, Edge>();
  const canvasNodes = useMemo<WorkflowCanvasNode[]>(
    () => [
      ...annotations.map((annotation) => ({
        ...annotation,
        zIndex: WORKFLOW_ANNOTATION_Z_INDEX,
      })),
      ...nodes.map((node) => ({
        ...node,
        // Notes reserve the bottom layer, while selected executable nodes keep
        // React Flow's usual elevation over their executable peers.
        zIndex: node.selected
          ? WORKFLOW_SELECTED_NODE_Z_INDEX
          : WORKFLOW_NODE_Z_INDEX,
      })),
    ],
    [annotations, nodes],
  );
  const reconnectingEdgeIdRef = useRef<string | null>(null);
  const edgeIdByDirectedPair = useMemo(() => {
    const pairs = new Map<string, string>();
    for (const edge of edges) {
      pairs.set(`${edge.source}\u0000${edge.target}`, edge.id);
    }
    return pairs;
  }, [edges]);

  /** Rejects self-loops and duplicate directed edges during connect and reconnect. */
  function isValidConnection(connection: Connection | Edge): boolean {
    if (
      connection.source === null ||
      connection.target === null ||
      connection.source === connection.target
    ) {
      return false;
    }
    const existingEdgeId = edgeIdByDirectedPair.get(
      `${connection.source}\u0000${connection.target}`,
    );
    return (
      existingEdgeId === undefined ||
      existingEdgeId === reconnectingEdgeIdRef.current
    );
  }

  const connectionState = useMemo(() => {
    return {
      connectionCandidateEndpoint:
        connectionCandidateNodeId === null
          ? null
          : connectionDraft?.kind === "new"
            ? ("target" as const)
            : (connectionDraft?.endpoint ?? null),
      connectionCandidateNodeId,
    };
  }, [connectionCandidateNodeId, connectionDraft]);

  useEffect(
    () => () => {
      if (connectionCandidateFrameRef.current !== null) {
        cancelAnimationFrame(connectionCandidateFrameRef.current);
      }
    },
    [],
  );

  useEffect(() => {
    // Version preview replaces the displayed graph without remounting the
    // history popover, so the viewport follows the selected graph directly.
    void setViewport(initialViewport);
  }, [initialViewport, setViewport]);

  /** Adds a note centered in the visible canvas rather than at the graph origin. */
  function addAnnotationAtViewportCenter(): void {
    const bounds = canvasRef.current?.getBoundingClientRect();
    if (bounds === undefined) {
      return;
    }
    const center = screenToFlowPosition({
      x: bounds.left + bounds.width / 2,
      y: bounds.top + bounds.height / 2,
    });
    onAddAnnotation(
      snapNodePosition({
        x: center.x - WORKFLOW_ANNOTATION_WIDTH / 2,
        y: center.y - WORKFLOW_ANNOTATION_HEIGHT / 2,
      }),
    );
  }

  /** Applies layout, then frames executable nodes after React Flow receives their positions. */
  function organizeAndFrameNodes(): void {
    onOrganize();
    requestAnimationFrame(() => {
      void fitView({
        nodes: nodes.map((node) => ({ id: node.id })),
        duration: 240,
        maxZoom: 1,
        minZoom: MIN_WORKFLOW_ZOOM,
        padding: 0.16,
      });
    });
  }

  /** Updates candidate state only when the actual card changes. */
  function commitConnectionCandidate(candidateNodeId: string | null): void {
    if (connectionCandidateNodeIdRef.current === candidateNodeId) {
      return;
    }
    connectionCandidateNodeIdRef.current = candidateNodeId;
    setConnectionCandidateNodeId(candidateNodeId);
  }

  /** Clears connection-only state after React Flow has completed or cancelled a gesture. */
  function finishConnectionGesture(): void {
    if (connectionCandidateFrameRef.current !== null) {
      cancelAnimationFrame(connectionCandidateFrameRef.current);
      connectionCandidateFrameRef.current = null;
    }
    connectionCandidatePointRef.current = null;
    setConnectionDraft(null);
    reconnectingEdgeIdRef.current = null;
    commitConnectionCandidate(null);
  }

  /**
   * Coalesces whole-card hit testing to one check per animation frame so React
   * Flow can update the preview endpoint before candidate detection does DOM work.
   */
  function updateConnectionCandidate(
    event: ReactPointerEvent<HTMLDivElement>,
  ): void {
    if (connectionDraft === null) {
      return;
    }
    connectionCandidatePointRef.current = {
      x: event.clientX,
      y: event.clientY,
    };
    if (connectionCandidateFrameRef.current !== null) {
      return;
    }
    connectionCandidateFrameRef.current = requestAnimationFrame(() => {
      connectionCandidateFrameRef.current = null;
      const draft = connectionDraft;
      const point = connectionCandidatePointRef.current;
      if (draft === null || point === null) {
        return;
      }
      const candidate = workflowNodeAtClientPoint(point.x, point.y);
      const validCandidate =
        candidate !== null &&
        isValidConnection(connectionForCandidate(draft, candidate))
          ? candidate
          : null;
      commitConnectionCandidate(validCandidate);
    });
  }

  /** Records a source drag so nearby cards can provide the original forgiving target. */
  function startConnection(params: OnConnectStartParams): void {
    // React Flow also emits the generic connection lifecycle while reconnecting.
    // The reconnect draft must remain authoritative or a moved endpoint becomes
    // an accidental new edge.
    if (
      reconnectingEdgeIdRef.current === null &&
      params.nodeId !== null &&
      params.handleType === "source"
    ) {
      setConnectionDraft({
        kind: "new",
        source: params.nodeId,
      });
    }
  }

  /** Commits a card drop when React Flow did not hit the card's smaller target handle. */
  function finishNewConnection(
    event: MouseEvent | TouchEvent,
    connectionState: FinalConnectionState,
  ): void {
    const draft = connectionDraft;
    // A reconnect has its own end callback. Clearing it from this generic
    // callback makes the later reconnect end look like a cancelled gesture.
    if (draft?.kind !== "new") {
      return;
    }
    const point = connectionEndClientPoint(event);
    if (connectionState.isValid !== true && point !== null) {
      const candidate = workflowNodeAtClientPoint(point.x, point.y);
      if (candidate !== null) {
        const connection = connectionForCandidate(draft, candidate);
        if (isValidConnection(connection)) {
          onConnect(connection);
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
    const draft = connectionDraft;
    const point = connectionEndClientPoint(event);
    if (
      connectionState.isValid !== true &&
      draft?.kind === "reconnect" &&
      point !== null
    ) {
      const candidate = workflowNodeAtClientPoint(point.x, point.y);
      if (candidate !== null) {
        const connection = connectionForCandidate(draft, candidate);
        if (isValidConnection(connection)) {
          onReconnect(edge, connection);
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
    position: XYPosition,
  ): void {
    const bounds = canvasRef.current?.getBoundingClientRect();
    if (
      bounds === undefined ||
      position.x < bounds.left ||
      position.x > bounds.right ||
      position.y < bounds.top ||
      position.y > bounds.bottom
    ) {
      return;
    }
    onAddNode(
      kind,
      snapNodePosition(
        nodePositionAt(
          screenToFlowPosition(
            {
              x: position.x,
              y: position.y,
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
  function guardPanelResizeEdge(
    event: ReactPointerEvent<HTMLDivElement>,
  ): void {
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
        data-workflow-edge-count={edges.length}
        data-workflow-node-count={nodes.length}
        onPointerDownCapture={guardPanelResizeEdge}
        onPointerMoveCapture={updateConnectionCandidate}
      >
        <WorkflowConnectionStateProvider value={connectionState}>
          <ReactFlow
            className="workflow-flow bg-muted/25"
            data-interaction-mode={interactionMode}
            nodes={canvasNodes}
            edges={edges}
            nodeTypes={nodeTypes}
            edgeTypes={edgeTypes}
            defaultViewport={initialViewport}
            minZoom={MIN_WORKFLOW_ZOOM}
            maxZoom={MAX_WORKFLOW_ZOOM}
            proOptions={{ hideAttribution: true }}
            nodesFocusable
            edgesFocusable
            nodesDraggable={!readOnly}
            nodesConnectable={!readOnly}
            elementsSelectable={!readOnly}
            elevateNodesOnSelect={false}
            edgesReconnectable={!readOnly}
            reconnectRadius={28}
            connectionRadius={24}
            deleteKeyCode={readOnly ? [] : ["Backspace", "Delete"]}
            multiSelectionKeyCode={null}
            snapGrid={WORKFLOW_SNAP_GRID}
            snapToGrid
            panOnScroll={false}
            zoomOnScroll
            zoomOnPinch
            // Left-drag box-selects multiple nodes; middle-drag keeps panning.
            panOnDrag={interactionMode === "hand" ? [0, 1] : [1]}
            selectionOnDrag={!readOnly && interactionMode === "pointer"}
            selectNodesOnDrag={false}
            isValidConnection={isValidConnection}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onNodeClick={(_event, node) => {
              // Selection alone cannot reopen the rail: drag-collapse keeps the
              // node selected, so a same-node click is a no-op for React Flow.
              if (
                node.type === WORKFLOW_FLOW_NODE_TYPE &&
                inspectorCollapsed &&
                inspectorAvailable
              ) {
                onExpandInspector();
              }
            }}
            onConnectStart={(_event, params) => {
              startConnection(params);
            }}
            onConnect={onConnect}
            onConnectEnd={finishNewConnection}
            onReconnectStart={(_event, edge, handleType) => {
              reconnectingEdgeIdRef.current = edge.id;
              setConnectionDraft({
                kind: "reconnect",
                edgeId: edge.id,
                // React Flow reports the fixed opposite handle here: dragging
                // the visible source endpoint therefore reports "target".
                endpoint: handleType === "target" ? "source" : "target",
                source: edge.source,
                target: edge.target,
              });
            }}
            onReconnect={onReconnect}
            onReconnectEnd={finishReconnect}
            onEdgeDoubleClick={(_event, edge) => {
              void deleteElements({ edges: [edge] });
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
            <WorkflowFlowOverview nodeCount={nodes.length} />
          </ReactFlow>
        </WorkflowConnectionStateProvider>

        {/* History caption sits in the same row as zoom so it cannot overlap the toolbar. */}
        <div className="pointer-events-none absolute inset-x-2 top-2 z-40 flex items-center gap-2">
          {inspectorCollapsed && inspectorAvailable && (
            <div className="pointer-events-auto">
              <WorkflowCanvasInspectorRestore
                onExpandInspector={onExpandInspector}
              />
            </div>
          )}
          <div className="pointer-events-auto ml-auto flex min-w-0 shrink-0 items-center gap-2">
            <WorkflowVersionHistory
              versions={versionHistory}
              previewedVersion={previewedVersion}
              activeVersion={activeVersion}
              draftUpdatedAt={draftUpdatedAt}
              onPreviewVersion={onPreviewVersion}
              onActivateVersion={onActivateVersion}
              onPublishDraft={onPublishDraft}
              onDeleteVersion={onDeleteVersion}
            />
            <WorkflowCanvasControls defaultViewport={DEFAULT_VIEWPORT} />
          </div>
        </div>
        <WorkflowCanvasTools
          mode={interactionMode}
          readOnly={readOnly}
          onModeChange={setInteractionMode}
          onAddAnnotation={addAnnotationAtViewportCenter}
          onOrganize={organizeAndFrameNodes}
        />
      </div>

      {!readOnly && (
        <div
          data-workflow-controls
          className="absolute bottom-3 left-1/2 z-30 w-fit max-w-[calc(100%-6rem)] -translate-x-1/2"
        >
          <WorkflowNodeCatalog
            capabilities={capabilities}
            hasStartNode={nodes.some((node) => node.data.kind === "start")}
            onAdd={addNodeAtViewportCenter}
            onDrop={dropNodeAtClientPosition}
          />
        </div>
      )}
    </div>
  );
}
