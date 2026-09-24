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
import {
  WORKFLOW_ITERATION_COLLAPSED_HEIGHT,
  WORKFLOW_ITERATION_COLLAPSED_WIDTH,
  type WorkflowNodeData,
  type WorkflowNodeKind,
} from "@ora/workflow-mock";
import { toast } from "@ora/ui";
import { WorkflowNodeCatalog } from "../workflow-node-catalog";
import {
  DEFAULT_WORKFLOW_PAN,
  DEFAULT_WORKFLOW_ZOOM,
  MAX_WORKFLOW_ZOOM,
  MIN_WORKFLOW_ZOOM,
} from "../../workflow-node-chrome/viewport";
import {
  WORKFLOW_FLOW_EDGE_TYPE,
  WORKFLOW_FLOW_NODE_TYPE,
  WORKFLOW_SNAP_GRID,
  containWorkflowCanvasNodes,
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
import { WorkflowHistoryControls } from "./history-controls";
import type { WorkflowCanvasNode, WorkflowCanvasProps } from "./types";
import { WorkflowVersionHistory } from "./version-history";
import {
  iterationExpandedSize,
  projectIterationEdges,
} from "../workflow-iteration-graph";
import { WorkflowIterationActionsProvider } from "./iteration-actions";
import {
  WORKFLOW_ANNOTATION_Z_INDEX,
  WORKFLOW_NODE_Z_INDEX,
  WORKFLOW_SELECTED_NODE_Z_INDEX,
} from "./z-index";
import {
  connectionForCandidate,
  reconnectDraft,
  type ConnectionDraft,
} from "./connection-gesture";
import { isValidWorkflowConnection } from "./connection-validation";
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

/**
 * Caches iteration presentation `data` across canvas projections. Module-scoped
 * (not a render ref) so `useMemo` can reuse entries without `react-hooks/refs`.
 */
const iterationPresentationDataCache = new Map<
  string,
  {
    source: WorkflowNodeData;
    count: number;
    data: WorkflowNodeData;
  }
>();

/**
 * Reuses projected RF node objects for undragged cards. Remapping `{...node}`
 * every pointer move breaks `memo` on Agent/Start views.
 */
const projectedCanvasNodeCache = new Map<
  string,
  {
    source: WorkflowCanvasNode;
    data: WorkflowNodeData | WorkflowCanvasNode["data"];
    extent: "parent" | undefined;
    expandParent: boolean | undefined;
    zIndex: number;
    hidden: boolean;
    projected: WorkflowCanvasNode;
  }
>();

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

/** Wraps the flow in a provider so catalog drop can convert screen coordinates. */
export function WorkflowCanvas(props: WorkflowCanvasProps) {
  return (
    <WorkflowAnnotationActionsProvider
      value={{
        readOnly: props.readOnly,
        update: props.onUpdateAnnotation,
        remove: props.onDeleteAnnotation,
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
  onInsertIterationNode,
  onToggleIterationCollapsed,
  onAddAnnotation,
  onOrganize,
  onConnect,
  onReconnect,
  onBeforeDelete,
  onDelete,
  onNodeDragStart,
  onNodeDragStop,
  canUndo,
  canRedo,
  historyPast,
  historyFuture,
  historyCurrentEvent,
  historyCurrentMeta,
  onUndo,
  onRedo,
  onHistoryJump,
  onClearHistory,
  inspectorCollapsed,
  inspectorAvailable,
  onExpandInspector,
  onConfigureGlobalVariables,
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
  // Collapsed iteration frames hide their region members: the members stay in the
  // graph (the frozen structure is authoritative); only the canvas presentation folds.
  const collapsedIterations = useMemo(() => {
    const collapsed = new Set<string>();
    for (const node of nodes) {
      if (node.data.kind === "iteration" && node.data.collapsed === true) {
        collapsed.add(node.id);
      }
    }
    return collapsed;
  }, [nodes]);
  // Region member counts feed the iteration frames' collapsed summary badge.
  const memberCountByIteration = useMemo(() => {
    const counts = new Map<string, number>();
    for (const node of nodes) {
      if (node.parentId !== undefined) {
        counts.set(node.parentId, (counts.get(node.parentId) ?? 0) + 1);
      }
    }
    return counts;
  }, [nodes]);
  const canvasNodes = useMemo<WorkflowCanvasNode[]>(() => {
    const iterationIds = new Set(
      nodes
        .filter((node) => node.data.kind === "iteration")
        .map((node) => node.id),
    );
    const presentationCache = iterationPresentationDataCache;
    const projectionCache = projectedCanvasNodeCache;
    const liveIds = new Set<string>();
    const executableNodes = containWorkflowCanvasNodes(nodes).map((node) => {
      liveIds.add(node.id);
      let data = node.data;
      if (node.data.kind === "iteration") {
        const count = memberCountByIteration.get(node.id) ?? 0;
        const cachedData = presentationCache.get(node.id);
        if (
          cachedData !== undefined &&
          cachedData.source === node.data &&
          cachedData.count === count
        ) {
          data = cachedData.data;
        } else {
          data = { ...node.data, regionMemberCount: count };
          presentationCache.set(node.id, {
            source: node.data,
            count,
            data,
          });
        }
      }
      // parentId is persisted graph structure; React Flow constraints are presentation only.
      // Loop bodies still use expandParent. Iteration members must not: frames already
      // grow through expandIterationFrames, and stacking both on nested regions fights
      // over parent size on every measurement and makes the canvas thrash.
      const extent =
        node.data.containerId !== undefined ||
        (node.parentId !== undefined && iterationIds.has(node.parentId))
          ? ("parent" as const)
          : undefined;
      const expandParent =
        node.data.containerId !== undefined ? true : undefined;
      // Notes reserve the bottom layer, while selected executable nodes keep
      // React Flow's usual elevation over their executable peers.
      const zIndex =
        node.data.kind === "loop"
          ? 0
          : node.selected
            ? WORKFLOW_SELECTED_NODE_Z_INDEX
            : WORKFLOW_NODE_Z_INDEX;
      const hidden =
        node.parentId !== undefined && collapsedIterations.has(node.parentId);
      const cached = projectionCache.get(node.id);
      if (
        cached !== undefined &&
        cached.source === node &&
        cached.data === data &&
        cached.extent === extent &&
        cached.expandParent === expandParent &&
        cached.zIndex === zIndex &&
        cached.hidden === hidden
      ) {
        return cached.projected;
      }
      const projected: WorkflowCanvasNode = {
        ...node,
        data,
        extent,
        expandParent,
        zIndex,
        ...(hidden ? { hidden: true } : { hidden: undefined }),
      };
      projectionCache.set(node.id, {
        source: node,
        data,
        extent,
        expandParent,
        zIndex,
        hidden,
        projected,
      });
      return projected;
    });
    for (const cachedId of presentationCache.keys()) {
      if (!liveIds.has(cachedId)) {
        presentationCache.delete(cachedId);
      }
    }
    const projectedAnnotations = annotations.map((annotation) => {
      liveIds.add(annotation.id);
      const cached = projectionCache.get(annotation.id);
      if (
        cached !== undefined &&
        cached.source === annotation &&
        cached.zIndex === WORKFLOW_ANNOTATION_Z_INDEX &&
        cached.hidden === false
      ) {
        return cached.projected;
      }
      const projected: WorkflowCanvasNode = {
        ...annotation,
        zIndex: WORKFLOW_ANNOTATION_Z_INDEX,
      };
      projectionCache.set(annotation.id, {
        source: annotation,
        data: annotation.data,
        extent: undefined,
        expandParent: undefined,
        zIndex: WORKFLOW_ANNOTATION_Z_INDEX,
        hidden: false,
        projected,
      });
      return projected;
    });
    for (const cachedId of projectionCache.keys()) {
      if (!liveIds.has(cachedId)) {
        projectionCache.delete(cachedId);
      }
    }
    return [
      ...projectedAnnotations,
      ...executableNodes.filter((node) => node.parentId === undefined),
      ...executableNodes.filter((node) => node.parentId !== undefined),
    ];
  }, [annotations, nodes, collapsedIterations, memberCountByIteration]);
  const canvasEdges = useMemo(
    () => projectIterationEdges({ nodes, edges }, collapsedIterations),
    [collapsedIterations, edges, nodes],
  );
  const reconnectingEdgeIdRef = useRef<string | null>(null);

  /** Rejects self-loops, duplicate directed edges, and edges that cross an iteration
   * region boundary in a direction the composite runtime cannot honor: a member's edge
   * must stay inside its region, and only the owner's internal-start handle may enter. */
  function isValidConnection(connection: Connection | Edge): boolean {
    return isValidWorkflowConnection({
      connection,
      nodes,
      edges,
      reconnectingEdgeId: reconnectingEdgeIdRef.current,
    });
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
        sourceHandle: params.handleId,
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
    const flowPoint = screenToFlowPosition(
      { x: position.x, y: position.y },
      { snapToGrid: false },
    );
    const droppedOverIteration = nodes.some((node) => {
      if (node.data.kind !== "iteration") {
        return false;
      }
      const size = iterationExpandedSize(node);
      const width =
        node.data.collapsed === true
          ? WORKFLOW_ITERATION_COLLAPSED_WIDTH
          : size.width;
      const height =
        node.data.collapsed === true
          ? WORKFLOW_ITERATION_COLLAPSED_HEIGHT
          : size.height;
      return (
        flowPoint.x >= node.position.x &&
        flowPoint.x <= node.position.x + width &&
        flowPoint.y >= node.position.y &&
        flowPoint.y <= node.position.y + height
      );
    });
    if (droppedOverIteration) {
      toast.message(t("settings.workflow.iteration.useInternalAdd"));
      return;
    }
    onAddNode(kind, snapNodePosition(nodePositionAt(flowPoint)));
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
          <WorkflowIterationActionsProvider
            capabilities={capabilities}
            nodes={nodes}
            edges={edges}
            readOnly={readOnly}
            onInsert={onInsertIterationNode}
            onToggleCollapsed={onToggleIterationCollapsed}
          >
            <ReactFlow
              className="workflow-flow bg-muted/25"
              data-interaction-mode={interactionMode}
              nodes={canvasNodes}
              edges={canvasEdges}
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
              // Auto-pan under a clamped iteration member moves the viewport while
              // the node sticks to the frame edge, so the card jitters relative to
              // the pointer. Users can still pan with the hand tool / middle drag.
              autoPanOnNodeDrag={false}
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
              onBeforeDelete={onBeforeDelete}
              onDelete={onDelete}
              onNodeDragStart={onNodeDragStart}
              onNodeDragStop={onNodeDragStop}
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
                setConnectionDraft(reconnectDraft(edge, handleType));
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
          </WorkflowIterationActionsProvider>
        </WorkflowConnectionStateProvider>

        {/* Version state remains top-right while viewport controls stay independently bottom-right. */}
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
          </div>
        </div>
        <WorkflowCanvasControls defaultViewport={DEFAULT_VIEWPORT} />
        <WorkflowCanvasTools
          mode={interactionMode}
          readOnly={readOnly}
          onModeChange={setInteractionMode}
          onConfigureGlobalVariables={onConfigureGlobalVariables}
          onAddAnnotation={addAnnotationAtViewportCenter}
          onOrganize={organizeAndFrameNodes}
        />
        <div className="absolute bottom-3 left-3 z-40">
          <WorkflowHistoryControls
            canUndo={canUndo}
            canRedo={canRedo}
            past={historyPast}
            future={historyFuture}
            currentEvent={historyCurrentEvent}
            currentMeta={historyCurrentMeta}
            readOnly={readOnly}
            onUndo={onUndo}
            onRedo={onRedo}
            onJump={onHistoryJump}
            onClear={onClearHistory}
          />
        </div>
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
