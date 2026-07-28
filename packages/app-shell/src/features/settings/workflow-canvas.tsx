import {
  useRef,
  useState,
  type DragEvent as ReactDragEvent,
  type ReactNode,
  type PointerEvent as ReactPointerEvent,
  type WheelEvent as ReactWheelEvent,
} from "react";
import { useTranslation } from "react-i18next";
import {
  IconFocusCentered,
  IconMinus,
  IconPlus,
  IconTrash,
} from "@tabler/icons-react";
import { Button, cn } from "@ora/ui";
import type {
  WorkflowEdge,
  WorkflowNode,
  WorkflowNodeKind,
  WorkflowPosition,
} from "@ora/workflow-mock";
import {
  getNodeMetadata,
  WORKFLOW_NODE_CATALOG,
  WORKFLOW_NODE_DRAG_DATA_TYPE,
} from "./workflow-node-metadata";
import {
  DEFAULT_WORKFLOW_PAN,
  DEFAULT_WORKFLOW_ZOOM,
  MAX_WORKFLOW_ZOOM,
  MIN_WORKFLOW_ZOOM,
  workflowWheelZoom,
  zoomWorkflowAtPoint,
  type WorkflowViewport,
} from "./workflow-viewport";

const STAGE_WIDTH = 2400;
const STAGE_HEIGHT = 1400;
const NODE_WIDTH = 230;
const NODE_ANCHOR_Y = 61;

interface WorkflowCanvasProps {
  children?: (onAddNode: (kind: WorkflowNodeKind) => void) => ReactNode;
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
}

type ConnectionDraft =
  | {
      kind: "new";
      source: string;
      pointer: WorkflowPosition;
      candidateNodeId: string | null;
    }
  | {
      kind: "reconnect";
      edgeId: string;
      endpoint: "source" | "target";
      fixedNodeId: string;
      pointer: WorkflowPosition;
      candidateNodeId: string | null;
    };

interface PanDraft {
  pointer: WorkflowPosition;
  pan: WorkflowPosition;
  moved: boolean;
}

/** Renders and manipulates the node graph without coupling it to persistence or preview behavior. */
export function WorkflowCanvas({
  children,
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
}: WorkflowCanvasProps) {
  const { t } = useTranslation();
  const canvasRef = useRef<HTMLElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const [connection, setConnection] = useState<ConnectionDraft | null>(null);
  const [viewport, setViewport] = useState<WorkflowViewport>({
    zoom: DEFAULT_WORKFLOW_ZOOM,
    pan: DEFAULT_WORKFLOW_PAN,
  });
  const [panDraft, setPanDraft] = useState<PanDraft | null>(null);
  const [nodeDropActive, setNodeDropActive] = useState(false);
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  const { zoom, pan } = viewport;
  const selectedEdge = edges.find((edge) => edge.id === selectedEdgeId);
  const activeSelectedEdgeId = selectedEdge?.id ?? null;

  /** Converts viewport pointer coordinates into stable graph coordinates at any zoom. */
  function graphPoint(clientX: number, clientY: number): WorkflowPosition {
    const bounds = canvasRef.current?.getBoundingClientRect();
    if (bounds === undefined) {
      return { x: 0, y: 0 };
    }
    return {
      x: (clientX - bounds.left - pan.x) / zoom,
      y: (clientY - bounds.top - pan.y) / zoom,
    };
  }

  /** Centers a new node around a graph point while keeping the card inside the stage. */
  function nodePositionAt(point: WorkflowPosition): WorkflowPosition {
    return {
      x: Math.min(
        STAGE_WIDTH - NODE_WIDTH - 16,
        Math.max(16, point.x - NODE_WIDTH / 2),
      ),
      y: Math.min(
        STAGE_HEIGHT - NODE_ANCHOR_Y * 2 - 16,
        Math.max(16, point.y - NODE_ANCHOR_Y),
      ),
    };
  }

  /** Adds a clicked catalog item to the center of the currently visible canvas. */
  function addNodeAtViewportCenter(kind: WorkflowNodeKind): void {
    const bounds = canvasRef.current?.getBoundingClientRect();
    const center = bounds === undefined
      ? { x: 0, y: 0 }
      : graphPoint(bounds.left + bounds.width / 2, bounds.top + bounds.height / 2);
    onAddNode(kind, nodePositionAt(center));
  }

  /** Reads a supported node kind without trusting arbitrary drag payloads. */
  function draggedNodeKind(event: ReactDragEvent<HTMLElement>): WorkflowNodeKind | null {
    const value = event.dataTransfer.getData(WORKFLOW_NODE_DRAG_DATA_TYPE);
    return WORKFLOW_NODE_CATALOG.some((item) => item.kind === value)
      ? value as WorkflowNodeKind
      : null;
  }

  /** Enables native drop feedback only for workflow node payloads. */
  function dragNodeOverCanvas(event: ReactDragEvent<HTMLElement>): void {
    if (!Array.from(event.dataTransfer.types).includes(WORKFLOW_NODE_DRAG_DATA_TYPE)) {
      return;
    }
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setNodeDropActive(true);
  }

  /** Creates a dragged node at the release point in graph coordinates. */
  function dropNodeOnCanvas(event: ReactDragEvent<HTMLElement>): void {
    const kind = draggedNodeKind(event);
    setNodeDropActive(false);
    if (kind === null) {
      return;
    }
    event.preventDefault();
    onAddNode(kind, nodePositionAt(graphPoint(event.clientX, event.clientY)));
  }

  /** Starts a connection preview and resolves the target beneath the pointer on release. */
  function startConnection(event: ReactPointerEvent, source: string): void {
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    setConnection({
      kind: "new",
      source,
      pointer: graphPoint(event.clientX, event.clientY),
      candidateNodeId: null,
    });
  }

  /** Detaches one selected endpoint while preserving the opposite end until a valid drop. */
  function startEdgeReconnect(
    event: ReactPointerEvent,
    edgeId: string,
    endpoint: "source" | "target",
    fixedNodeId: string,
  ): void {
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    setConnection({
      kind: "reconnect",
      edgeId,
      endpoint,
      fixedNodeId,
      pointer: graphPoint(event.clientX, event.clientY),
      candidateNodeId: null,
    });
  }

  /** Resolves a whole node card as a forgiving drop zone while preserving port direction. */
  function connectionCandidate(
    draft: ConnectionDraft,
    clientX: number,
    clientY: number,
  ): string | null {
    const element = document.elementFromPoint(clientX, clientY);
    const endpoint = draft.kind === "new" ? "target" : draft.endpoint;
    const portNodeId = endpoint === "source"
      ? element?.closest<HTMLElement>("[data-workflow-output]")?.dataset.workflowOutput
      : element?.closest<HTMLElement>("[data-workflow-input]")?.dataset.workflowInput;
    const candidateNodeId = portNodeId
      ?? element
        ?.closest<HTMLElement>("[data-workflow-node-id]")
        ?.dataset.workflowNodeId;
    if (candidateNodeId === undefined) {
      return null;
    }

    const source = draft.kind === "new"
      ? draft.source
      : endpoint === "source"
        ? candidateNodeId
        : draft.fixedNodeId;
    const target = draft.kind === "new"
      ? candidateNodeId
      : endpoint === "target"
        ? candidateNodeId
        : draft.fixedNodeId;
    const editedEdgeId = draft.kind === "reconnect" ? draft.edgeId : null;
    if (
      source === target
      || edges.some(
        (edge) =>
          edge.id !== editedEdgeId
          && edge.source === source
          && edge.target === target,
      )
    ) {
      return null;
    }
    return candidateNodeId;
  }

  /** Updates the preview and candidate highlight in graph space while zoomed. */
  function moveConnection(event: ReactPointerEvent): void {
    if (connection === null) {
      return;
    }
    setConnection({
      ...connection,
      pointer: graphPoint(event.clientX, event.clientY),
      candidateNodeId: connectionCandidate(
        connection,
        event.clientX,
        event.clientY,
      ),
    });
  }

  /** Commits new and edited links only when released over the matching node port. */
  function finishConnection(event: ReactPointerEvent): void {
    if (connection === null) {
      return;
    }
    const candidateNodeId = connectionCandidate(
      connection,
      event.clientX,
      event.clientY,
    );
    if (connection.kind === "new") {
      if (candidateNodeId !== null) {
        onConnect(connection.source, candidateNodeId);
      }
    } else if (connection.endpoint === "target") {
      if (candidateNodeId !== null) {
        onReconnectEdge(
          connection.edgeId,
          connection.fixedNodeId,
          candidateNodeId,
        );
      }
    } else if (candidateNodeId !== null) {
      onReconnectEdge(
        connection.edgeId,
        candidateNodeId,
        connection.fixedNodeId,
      );
    }
    setConnection(null);
  }

  /** Zooms around the pointer rather than the viewport center to preserve spatial context. */
  function handleWheel(event: ReactWheelEvent<HTMLElement>): void {
    event.preventDefault();
    const bounds = canvasRef.current?.getBoundingClientRect();
    if (bounds === undefined) {
      return;
    }
    const cursor = {
      x: event.clientX - bounds.left,
      y: event.clientY - bounds.top,
    };
    setViewport((current) =>
      zoomWorkflowAtPoint(
        current,
        workflowWheelZoom(current.zoom, event.deltaY),
        cursor,
      ),
    );
  }

  /** Starts board panning only from empty space so node and connection gestures remain independent. */
  function startPanning(event: ReactPointerEvent<HTMLElement>): void {
    if (event.button !== 0 && event.button !== 1) {
      return;
    }
    const target = event.target as HTMLElement;
    if (
      target.closest(
        "[data-workflow-node], [data-workflow-edge], [data-workflow-controls], button, input, textarea, [role=combobox]",
      ) !== null
    ) {
      return;
    }
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    setPanDraft({
      pointer: { x: event.clientX, y: event.clientY },
      pan,
      moved: false,
    });
  }

  /** Tracks pan from its pointer-down origin so fast movement never accumulates rounding drift. */
  function movePanning(event: ReactPointerEvent<HTMLElement>): void {
    if (panDraft === null) {
      return;
    }
    const delta = {
      x: event.clientX - panDraft.pointer.x,
      y: event.clientY - panDraft.pointer.y,
    };
    setViewport((current) => ({
      ...current,
      pan: {
        x: panDraft.pan.x + delta.x,
        y: panDraft.pan.y + delta.y,
      },
    }));
    if (!panDraft.moved && Math.hypot(delta.x, delta.y) >= 3) {
      setPanDraft({ ...panDraft, moved: true });
    }
  }

  /** Ends panning and treats a stationary background press as canvas deselection. */
  function finishPanning(): void {
    if (panDraft !== null && !panDraft.moved) {
      onSelectNode(null);
      setSelectedEdgeId(null);
    }
    setPanDraft(null);
  }

  /** Applies toolbar zoom around the canvas center as a predictable button alternative. */
  function zoomFromCenter(nextZoom: number): void {
    const bounds = canvasRef.current?.getBoundingClientRect();
    const cursor = bounds === undefined
      ? { x: 0, y: 0 }
      : { x: bounds.width / 2, y: bounds.height / 2 };
    setViewport((current) => zoomWorkflowAtPoint(current, nextZoom, cursor));
  }

  /** Deletes the selected graph element while leaving unrelated workflow state intact. */
  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>): void {
    if (event.key !== "Delete" && event.key !== "Backspace") {
      return;
    }
    if (activeSelectedEdgeId !== null) {
      event.preventDefault();
      onDeleteEdge(activeSelectedEdgeId);
      setSelectedEdgeId(null);
    } else if (selectedNodeId !== null) {
      event.preventDefault();
      onDeleteNode(selectedNodeId);
    }
  }

  return (
    <section
      ref={canvasRef}
      className={cn(
        "relative min-h-0 min-w-0 touch-none overflow-hidden bg-muted/25 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
        panDraft === null ? "cursor-grab" : "cursor-grabbing",
      )}
      aria-label={t("settings.workflow.canvas")}
      tabIndex={0}
      onKeyDown={handleKeyDown}
      onWheel={handleWheel}
      onPointerDown={startPanning}
      onPointerMove={movePanning}
      onPointerUp={finishPanning}
      onPointerCancel={finishPanning}
      onDragEnter={dragNodeOverCanvas}
      onDragOver={dragNodeOverCanvas}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
          setNodeDropActive(false);
        }
      }}
      onDrop={dropNodeOnCanvas}
      style={{
        backgroundImage:
          "radial-gradient(circle, color-mix(in oklch, var(--foreground) 18%, transparent) 1px, transparent 1px)",
        backgroundPosition: `${pan.x}px ${pan.y}px`,
        backgroundSize: `${20 * zoom}px ${20 * zoom}px`,
      }}
    >
      <div
        ref={stageRef}
        className="absolute left-0 top-0 origin-top-left"
        style={{
          width: STAGE_WIDTH,
          height: STAGE_HEIGHT,
          transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
        }}
        onPointerMove={moveConnection}
        onPointerUp={finishConnection}
      >
        <WorkflowEdges
          nodes={nodes}
          edges={edges}
          connection={connection}
          selectedEdgeId={activeSelectedEdgeId}
          onSelectEdge={(edgeId) => {
            setSelectedEdgeId(edgeId);
            onSelectNode(null);
          }}
          onDeleteEdge={(edgeId) => {
            onDeleteEdge(edgeId);
            setSelectedEdgeId(null);
          }}
        />
        {nodes.map((node) => (
          <WorkflowNodeCard
            key={node.id}
            node={node}
            selected={selectedNodeId === node.id}
            zoom={zoom}
            onSelect={() => {
              setSelectedEdgeId(null);
              onSelectNode(node.id);
            }}
            onMove={(position) => onMoveNode(node.id, position)}
            onStartConnection={(event) => startConnection(event, node.id)}
            connectionCandidate={connection?.candidateNodeId === node.id
              ? connection.kind === "new"
                ? "target"
                : connection.endpoint
              : null}
            connectionEndpoint={selectedEdge === undefined
              ? null
              : selectedEdge.source === node.id
                ? "source"
                : selectedEdge.target === node.id
                  ? "target"
                  : null}
            onStartReconnect={(event, endpoint) => {
              if (selectedEdge === undefined) {
                return;
              }
              startEdgeReconnect(
                event,
                selectedEdge.id,
                endpoint,
                endpoint === "source" ? selectedEdge.target : selectedEdge.source,
              );
            }}
            onDelete={() => onDeleteNode(node.id)}
          />
        ))}
      </div>
      {nodeDropActive && (
        <div className="pointer-events-none absolute inset-3 z-20 rounded-xl border-2 border-dashed border-ring/55 bg-ring/[0.03]">
          <span className="absolute left-1/2 top-4 -translate-x-1/2 rounded-full border border-ring/20 bg-background/95 px-3 py-1 text-[10px] font-medium text-foreground shadow-sm">
            {t("settings.workflow.dropNode")}
          </span>
        </div>
      )}
      {children !== undefined && (
        <div
          data-workflow-controls
          className="absolute bottom-3 left-1/2 z-30 w-fit max-w-[calc(100%_-_12rem)] -translate-x-1/2"
        >
          {children(addNodeAtViewportCenter)}
        </div>
      )}
      <div
        data-workflow-controls
        className="absolute right-3 top-3 z-30 flex w-fit items-center rounded-lg border border-border/80 bg-background/95 p-px shadow-sm backdrop-blur"
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
          onClick={() =>
            setViewport({
              zoom: DEFAULT_WORKFLOW_ZOOM,
              pan: DEFAULT_WORKFLOW_PAN,
            })
          }
        >
          <IconFocusCentered />
        </Button>
        <span className="sr-only">
          {t("settings.workflow.canvasHint")}
        </span>
      </div>
    </section>
  );
}

/** Draws graph edges behind nodes so handles and cards stay visually dominant. */
function WorkflowEdges({
  nodes,
  edges,
  connection,
  selectedEdgeId,
  onSelectEdge,
  onDeleteEdge,
}: {
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
  connection: ConnectionDraft | null;
  selectedEdgeId: string | null;
  onSelectEdge: (edgeId: string) => void;
  onDeleteEdge: (edgeId: string) => void;
}) {
  const { t } = useTranslation();
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  return (
    <svg
      className="pointer-events-none absolute inset-0 z-0 overflow-visible"
      width={STAGE_WIDTH}
      height={STAGE_HEIGHT}
      aria-label={t("settings.workflow.connections")}
    >
      <defs>
        <marker id="workflow-arrow" markerWidth="7" markerHeight="7" refX="6" refY="3.5" orient="auto">
          <path d="M0,0 L7,3.5 L0,7 Z" className="fill-muted-foreground/70" />
        </marker>
        <marker id="workflow-arrow-selected" markerWidth="7" markerHeight="7" refX="6" refY="3.5" orient="auto">
          <path d="M0,0 L7,3.5 L0,7 Z" className="fill-ring" />
        </marker>
      </defs>
      {edges.map((edge) => {
        const source = nodeById.get(edge.source);
        const target = nodeById.get(edge.target);
        if (source === undefined || target === undefined) {
          return null;
        }
        const selected = selectedEdgeId === edge.id;
        const reconnecting = connection?.kind === "reconnect"
          && connection.edgeId === edge.id;
        const start = { x: source.position.x + NODE_WIDTH, y: source.position.y + NODE_ANCHOR_Y };
        const end = { x: target.position.x, y: target.position.y + NODE_ANCHOR_Y };
        const path = edgePath(start, end);
        const accessibleName = t("settings.workflow.selectConnection", {
          source: source.title,
          target: target.title,
        });
        return (
          <g key={edge.id}>
            <path
              d={path}
              fill="none"
              stroke={selected
                ? "var(--ring)"
                : "color-mix(in oklch, var(--foreground) 34%, transparent)"}
              strokeWidth={selected ? "3" : "2"}
              markerEnd={selected
                ? "url(#workflow-arrow-selected)"
                : "url(#workflow-arrow)"}
              className={cn(
                "transition-[stroke,stroke-width,opacity] duration-150 motion-reduce:transition-none",
                reconnecting && "opacity-0",
              )}
            />
            <path
              data-workflow-edge={edge.id}
              d={path}
              fill="none"
              stroke="transparent"
              strokeWidth="16"
              className="pointer-events-auto cursor-pointer outline-none"
              aria-label={accessibleName}
              aria-keyshortcuts="Delete Backspace"
              role="button"
              tabIndex={0}
              onClick={(event) => {
                event.stopPropagation();
                onSelectEdge(edge.id);
              }}
              onDoubleClick={(event) => {
                event.stopPropagation();
                onDeleteEdge(edge.id);
              }}
              onFocus={() => onSelectEdge(edge.id)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  onSelectEdge(edge.id);
                }
              }}
            />
            {edge.label !== undefined && (
              <text
                x={(start.x + end.x) / 2}
                y={(start.y + end.y) / 2 - 8}
                textAnchor="middle"
                className="fill-muted-foreground text-[10px]"
              >
                {edge.label}
              </text>
            )}
          </g>
        );
      })}
      {connection !== null && (() => {
        const candidate = connection.candidateNodeId === null
          ? undefined
          : nodeById.get(connection.candidateNodeId);
        const source = connection.kind === "new"
          ? nodeById.get(connection.source)
          : connection.endpoint === "target"
            ? nodeById.get(connection.fixedNodeId)
            : candidate;
        const target = connection.kind === "reconnect" && connection.endpoint === "source"
          ? nodeById.get(connection.fixedNodeId)
          : candidate;
        const start = source === undefined
          ? connection.pointer
          : {
              x: source.position.x + NODE_WIDTH,
              y: source.position.y + NODE_ANCHOR_Y,
            };
        const end = target === undefined
          ? connection.pointer
          : {
              x: target.position.x,
              y: target.position.y + NODE_ANCHOR_Y,
            };
        return (
          <path
            d={edgePath(start, end)}
            fill="none"
            stroke="var(--ring)"
            strokeDasharray="5 4"
            strokeWidth="2"
          />
        );
      })()}
    </svg>
  );
}

/** Builds a smooth horizontal curve that remains readable when nodes are vertically offset. */
function edgePath(start: WorkflowPosition, end: WorkflowPosition): string {
  const distance = Math.max(64, Math.abs(end.x - start.x) * 0.45);
  return `M ${start.x} ${start.y} C ${start.x + distance} ${start.y}, ${end.x - distance} ${end.y}, ${end.x} ${end.y}`;
}

/** Encapsulates node selection, dragging, and connection handles as one keyboard-focusable unit. */
function WorkflowNodeCard({
  node,
  selected,
  zoom,
  onSelect,
  onMove,
  onStartConnection,
  connectionCandidate,
  connectionEndpoint,
  onStartReconnect,
  onDelete,
}: {
  node: WorkflowNode;
  selected: boolean;
  zoom: number;
  onSelect: () => void;
  onMove: (position: WorkflowPosition) => void;
  onStartConnection: (event: ReactPointerEvent) => void;
  connectionCandidate: "source" | "target" | null;
  connectionEndpoint: "source" | "target" | null;
  onStartReconnect: (
    event: ReactPointerEvent,
    endpoint: "source" | "target",
  ) => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  const metadata = getNodeMetadata(node.kind);
  const nodeKindLabel = t(metadata.labelKey);
  const Icon = metadata.icon;
  const dragOrigin = useRef<{ pointer: WorkflowPosition; node: WorkflowPosition } | null>(null);

  /** Moves a node from its original graph-space position to avoid cumulative pointer rounding. */
  function handlePointerMove(event: ReactPointerEvent): void {
    if (dragOrigin.current === null) {
      return;
    }
    onMove({
      x: Math.max(16, dragOrigin.current.node.x + (event.clientX - dragOrigin.current.pointer.x) / zoom),
      y: Math.max(16, dragOrigin.current.node.y + (event.clientY - dragOrigin.current.pointer.y) / zoom),
    });
  }

  return (
    <article
      data-workflow-node
      data-workflow-node-id={node.id}
      className={cn(
        "absolute z-10 w-[230px] cursor-move rounded-xl border bg-card shadow-sm outline-none transition-[border-color,box-shadow] duration-200",
        connectionCandidate !== null
          ? "border-ring shadow-md ring-2 ring-ring/30"
          : selected
          ? "border-foreground/45 shadow-md ring-2 ring-ring/25"
          : "border-border hover:border-foreground/25 hover:shadow-md",
      )}
      style={{ left: node.position.x, top: node.position.y }}
      tabIndex={0}
      aria-label={`${t("settings.workflow.nodeSuffix", { type: nodeKindLabel })}: ${node.title}`}
      onFocus={onSelect}
      onPointerDown={(event) => {
        if ((event.target as HTMLElement).closest("button") !== null) {
          return;
        }
        onSelect();
        event.currentTarget.setPointerCapture(event.pointerId);
        dragOrigin.current = {
          pointer: { x: event.clientX, y: event.clientY },
          node: node.position,
        };
      }}
      onPointerMove={handlePointerMove}
      onPointerUp={() => {
        dragOrigin.current = null;
      }}
    >
      <button
        type="button"
        data-workflow-input={node.id}
        aria-label={connectionEndpoint === "target"
          ? t("settings.workflow.moveConnectionTarget", { name: node.title })
          : t("settings.workflow.connectTo", { name: node.title })}
        onPointerDown={connectionEndpoint === "target"
          ? (event) => onStartReconnect(event, "target")
          : undefined}
        className={cn(
          "absolute -left-3 top-[49px] flex size-6 items-center justify-center rounded-full outline-none after:size-3 after:rounded-full after:border-2 after:border-background after:bg-muted-foreground after:shadow-sm after:transition-transform hover:after:scale-125 focus-visible:ring-2 focus-visible:ring-ring",
          connectionEndpoint === "target" && "cursor-grab bg-ring/15 ring-2 ring-ring/35 after:scale-125 after:bg-ring active:cursor-grabbing",
          connectionCandidate === "target" && "bg-ring/20 ring-2 ring-ring after:scale-150 after:bg-ring",
        )}
      >
        <span className="sr-only">
          {connectionEndpoint === "target"
            ? t("settings.workflow.moveConnectionTarget", { name: node.title })
            : t("settings.workflow.connectTo", { name: node.title })}
        </span>
      </button>
      <div className="flex items-start gap-2.5 border-b border-border px-3 py-3">
        <span className={cn("flex size-8 shrink-0 items-center justify-center rounded-lg", metadata.tone)}>
          <Icon className="size-4" stroke={1.9} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <h4 className="truncate text-xs font-semibold">{node.title}</h4>
            <span className="rounded bg-muted px-1.5 py-0.5 text-[9px] font-medium text-muted-foreground">
              {nodeKindLabel}
            </span>
          </div>
          <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-muted-foreground">
            {node.description}
          </p>
        </div>
        {selected && (
          <button
            type="button"
            aria-label={t("settings.workflow.deleteNamed", { name: node.title })}
            onClick={onDelete}
            className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-destructive/10 hover:text-destructive focus-visible:ring-2 focus-visible:ring-ring"
          >
            <IconTrash className="size-3.5" />
          </button>
        )}
      </div>
      <div className="flex items-center justify-between px-3 py-2 text-[10px] text-muted-foreground">
        <span className="truncate">
          {node.config.model
            ?? node.config.source
            ?? node.config.language
            ?? node.config.tool
            ?? node.config.trigger
            ?? t("settings.workflow.immediate")}
        </span>
        <span className="font-mono text-[9px]">{node.id}</span>
      </div>
      <button
        type="button"
        data-workflow-output={node.id}
        aria-label={connectionEndpoint === "source"
          ? t("settings.workflow.moveConnectionSource", { name: node.title })
          : t("settings.workflow.connectFrom", { name: node.title })}
        onPointerDown={connectionEndpoint === "source"
          ? (event) => onStartReconnect(event, "source")
          : onStartConnection}
        className={cn(
          "absolute -right-3 top-[49px] flex size-6 items-center justify-center rounded-full outline-none after:size-3 after:rounded-full after:border-2 after:border-background after:bg-foreground after:shadow-sm after:transition-transform hover:after:scale-125 focus-visible:ring-2 focus-visible:ring-ring",
          connectionEndpoint === "source" && "cursor-grab bg-ring/15 ring-2 ring-ring/35 after:scale-125 after:bg-ring active:cursor-grabbing",
          connectionCandidate === "source" && "bg-ring/20 ring-2 ring-ring after:scale-150 after:bg-ring",
        )}
      >
        <span className="sr-only">
          {connectionEndpoint === "source"
            ? t("settings.workflow.moveConnectionSource", { name: node.title })
            : t("settings.workflow.connectFrom", { name: node.title })}
        </span>
      </button>
    </article>
  );
}
