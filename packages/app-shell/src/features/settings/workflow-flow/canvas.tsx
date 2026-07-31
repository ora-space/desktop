import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useTranslation } from "react-i18next";
import {
  Background,
  BackgroundVariant,
  MarkerType,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  useStore,
  type Connection,
  type Edge,
  type EdgeChange,
  type NodeChange,
  type Viewport,
} from "@xyflow/react";
import {
  IconFocusCentered,
  IconLayoutSidebarLeftExpand,
  IconLayoutSidebarRightExpand,
  IconMinus,
  IconPlus,
} from "@tabler/icons-react";
import { Button } from "@ora/ui";
import type {
  WorkflowEdge,
  WorkflowNode,
  WorkflowNodeKind,
  WorkflowPosition,
} from "@ora/workflow-mock";
import { WorkflowNodeCatalog } from "../workflow-node-catalog";
import {
  DEFAULT_WORKFLOW_PAN,
  DEFAULT_WORKFLOW_ZOOM,
  MAX_WORKFLOW_ZOOM,
  MIN_WORKFLOW_ZOOM,
  clampWorkflowZoom,
} from "../workflow-viewport";
import {
  WORKFLOW_FLOW_EDGE_TYPE,
  WORKFLOW_FLOW_NODE_TYPE,
  nodePositionAt,
  toFlowEdges,
  toFlowNodes,
} from "./adapters";
import { WorkflowFlowCallbacksProvider } from "./callbacks";
import { WorkflowFlowEdgeView } from "./edge";
import { WorkflowFlowNodeView } from "./node";
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

interface WorkflowCanvasProps {
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

interface ClientPosition {
  clientX: number;
  clientY: number;
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
  const reconnectingEdgeIdRef = useRef<string | null>(null);
  const { screenToFlowPosition, setViewport, getViewport, zoomTo } = useReactFlow();
  const zoom = useStore((state) => state.transform[2]);
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  const flowCallbacks = useMemo(
    () => ({
      onDeleteNode,
      onDeleteEdge,
      onSelectEdge: (edgeId: string) => {
        setSelectedEdgeId(edgeId);
        onSelectNode(null);
      },
    }),
    [onDeleteEdge, onDeleteNode, onSelectNode],
  );

  const flowNodes = useMemo(
    () => toFlowNodes(nodes, selectedNodeId),
    [nodes, selectedNodeId],
  );
  const flowEdges = useMemo(
    () => toFlowEdges(edges, nodes, selectedEdgeId),
    [edges, nodes, selectedEdgeId],
  );

  useEffect(() => {
    if (
      selectedEdgeId !== null
      && !edges.some((edge) => edge.id === selectedEdgeId)
    ) {
      setSelectedEdgeId(null);
    }
  }, [edges, selectedEdgeId]);

  /** Rejects self-loops and duplicate directed pairs so graph edits stay unambiguous. */
  const isValidConnection = useCallback(
    (connection: Connection | Edge) => {
      const source = connection.source;
      const target = connection.target;
      if (source === null || target === null || source === target) {
        return false;
      }
      const reconnectingEdgeId = reconnectingEdgeIdRef.current;
      return !edges.some(
        (edge) =>
          edge.id !== reconnectingEdgeId
          && edge.source === source
          && edge.target === target,
      );
    },
    [edges],
  );

  /** Forwards drag position updates only; ignore init/measure noise that would loop setState. */
  const handleNodesChange = useCallback(
    (changes: NodeChange[]) => {
      for (const change of changes) {
        if (
          change.type === "position"
          && change.position !== undefined
          && change.dragging !== undefined
        ) {
          onMoveNode(change.id, change.position);
        } else if (change.type === "remove") {
          onDeleteNode(change.id);
        }
      }
    },
    [onDeleteNode, onMoveNode],
  );

  /** Applies edge deletions while selection stays parent/local controlled via click handlers. */
  const handleEdgesChange = useCallback(
    (changes: EdgeChange[]) => {
      for (const change of changes) {
        if (change.type === "remove") {
          onDeleteEdge(change.id);
          setSelectedEdgeId((current) => (current === change.id ? null : current));
        }
      }
    },
    [onDeleteEdge],
  );

  /** Commits a new domain edge after React Flow validates the connection handles. */
  const handleConnect = useCallback(
    (connection: Connection) => {
      if (connection.source === null || connection.target === null) {
        return;
      }
      onConnect(connection.source, connection.target);
    },
    [onConnect],
  );

  /** Updates one existing edge when a reconnect drag lands on a valid endpoint. */
  const handleReconnect = useCallback(
    (oldEdge: Edge, connection: Connection) => {
      if (connection.source === null || connection.target === null) {
        return;
      }
      onReconnectEdge(oldEdge.id, connection.source, connection.target);
    },
    [onReconnectEdge],
  );

  /** Adds a clicked catalog item to the center of the currently visible canvas. */
  function addNodeAtViewportCenter(kind: WorkflowNodeKind): void {
    const bounds = canvasRef.current?.getBoundingClientRect();
    if (bounds === undefined) {
      onAddNode(kind, nodePositionAt({ x: 0, y: 0 }));
      return;
    }
    const point = screenToFlowPosition({
      x: bounds.left + bounds.width / 2,
      y: bounds.top + bounds.height / 2,
    });
    onAddNode(kind, nodePositionAt(point));
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
      nodePositionAt(
        screenToFlowPosition({
          x: position.clientX,
          y: position.clientY,
        }),
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

  /** Applies toolbar zoom around the viewport center as a predictable button alternative. */
  function zoomFromCenter(nextZoom: number): void {
    const clamped = clampWorkflowZoom(nextZoom);
    const bounds = canvasRef.current?.getBoundingClientRect();
    if (bounds === undefined) {
      zoomTo(clamped);
      return;
    }
    const viewport = getViewport();
    const cursor = { x: bounds.width / 2, y: bounds.height / 2 };
    const worldPoint = {
      x: (cursor.x - viewport.x) / viewport.zoom,
      y: (cursor.y - viewport.y) / viewport.zoom,
    };
    setViewport({
      zoom: clamped,
      x: cursor.x - worldPoint.x * clamped,
      y: cursor.y - worldPoint.y * clamped,
    });
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
      >
        <WorkflowFlowCallbacksProvider value={flowCallbacks}>
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
            deleteKeyCode={["Backspace", "Delete"]}
            multiSelectionKeyCode={null}
            panOnScroll={false}
            zoomOnScroll
            zoomOnPinch
            panOnDrag
            selectNodesOnDrag={false}
            isValidConnection={isValidConnection}
            onNodesChange={handleNodesChange}
            onEdgesChange={handleEdgesChange}
            onConnect={handleConnect}
            onReconnectStart={(_event, edge) => {
              reconnectingEdgeIdRef.current = edge.id;
            }}
            onReconnect={handleReconnect}
            onReconnectEnd={() => {
              reconnectingEdgeIdRef.current = null;
            }}
            onNodeClick={(_event, node) => {
              setSelectedEdgeId(null);
              onSelectNode(node.id);
            }}
            onEdgeClick={(_event, edge) => {
              setSelectedEdgeId(edge.id);
              onSelectNode(null);
            }}
            onPaneClick={() => {
              onSelectNode(null);
              setSelectedEdgeId(null);
            }}
            onEdgeDoubleClick={(_event, edge) => {
              onDeleteEdge(edge.id);
              setSelectedEdgeId(null);
            }}
            defaultEdgeOptions={{
              type: WORKFLOW_FLOW_EDGE_TYPE,
              markerEnd: {
                type: MarkerType.ArrowClosed,
                width: 7,
                height: 7,
                color: "color-mix(in oklch, var(--foreground) 70%, transparent)",
              },
            }}
            connectionLineStyle={{
              stroke: "var(--ring)",
              strokeWidth: 2,
              strokeDasharray: "5 4",
            }}
          >
            <Background
              id="workflow-dots"
              variant={BackgroundVariant.Dots}
              gap={20}
              size={1}
              color="color-mix(in oklch, var(--foreground) 18%, transparent)"
            />
          </ReactFlow>
        </WorkflowFlowCallbacksProvider>

        <div
          data-workflow-controls
          className="pointer-events-auto absolute right-2 top-2 z-30 flex w-fit items-center rounded-lg border border-border/80 bg-background/95 p-px shadow-sm backdrop-blur"
          aria-label={t("settings.workflow.canvasControls")}
          aria-orientation="horizontal"
          role="toolbar"
        >
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-7 rounded-md"
            aria-label={t("settings.workflow.zoomOut")}
            disabled={zoom <= MIN_WORKFLOW_ZOOM}
            onClick={() => zoomFromCenter(zoom - 0.1)}
          >
            <IconMinus />
          </Button>
          <span
            className="flex h-7 w-8 items-center justify-center text-[9px] font-medium tabular-nums text-muted-foreground"
            aria-live="polite"
          >
            {Math.round(zoom * 100)}%
          </span>
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-7 rounded-md"
            aria-label={t("settings.workflow.zoomIn")}
            disabled={zoom >= MAX_WORKFLOW_ZOOM}
            onClick={() => zoomFromCenter(zoom + 0.1)}
          >
            <IconPlus />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-7 rounded-md"
            aria-label={t("settings.workflow.resetView")}
            onClick={() => {
              setViewport(DEFAULT_VIEWPORT);
            }}
          >
            <IconFocusCentered />
          </Button>
          <span className="sr-only">
            {t("settings.workflow.canvasHint")}
          </span>
        </div>

        {(libraryCollapsed || inspectorCollapsed) && (
          <div
            data-workflow-controls
            className="pointer-events-auto absolute left-2 top-2 z-30 flex items-center gap-px rounded-lg border border-border/80 bg-background/95 p-px shadow-sm backdrop-blur"
          >
            {libraryCollapsed && (
              <Button
                variant="ghost"
                size="icon-sm"
                className="size-7 rounded-md"
                aria-label={t("settings.workflow.expandLibrary")}
                onClick={onExpandLibrary}
              >
                <IconLayoutSidebarLeftExpand />
              </Button>
            )}
            {inspectorCollapsed && inspectorAvailable && (
              <Button
                variant="ghost"
                size="icon-sm"
                className="size-7 rounded-md"
                aria-label={t("settings.workflow.expandConfiguration")}
                onClick={onExpandInspector}
              >
                <IconLayoutSidebarRightExpand />
              </Button>
            )}
          </div>
        )}
      </div>

      <div
        data-workflow-controls
        className="absolute bottom-3 left-1/2 z-30 w-fit max-w-[calc(100%_-_12rem)] -translate-x-1/2"
      >
        <WorkflowNodeCatalog
          onAdd={addNodeAtViewportCenter}
          onDrop={dropNodeAtClientPosition}
        />
      </div>
    </div>
  );
}
