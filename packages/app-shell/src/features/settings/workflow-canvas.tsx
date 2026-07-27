import {
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type WheelEvent as ReactWheelEvent,
} from "react";
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
  WorkflowPosition,
} from "@ora/workflow-mock";
import { getNodeMetadata } from "./workflow-node-metadata";
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
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string | null) => void;
  onMoveNode: (nodeId: string, position: WorkflowPosition) => void;
  onConnect: (source: string, target: string) => void;
  onDeleteNode: (nodeId: string) => void;
}

interface ConnectionDraft {
  source: string;
  pointer: WorkflowPosition;
}

interface PanDraft {
  pointer: WorkflowPosition;
  pan: WorkflowPosition;
  moved: boolean;
}

/** Renders and manipulates the node graph without coupling it to persistence or preview behavior. */
export function WorkflowCanvas({
  nodes,
  edges,
  selectedNodeId,
  onSelectNode,
  onMoveNode,
  onConnect,
  onDeleteNode,
}: WorkflowCanvasProps) {
  const canvasRef = useRef<HTMLElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const [connection, setConnection] = useState<ConnectionDraft | null>(null);
  const [viewport, setViewport] = useState<WorkflowViewport>({
    zoom: DEFAULT_WORKFLOW_ZOOM,
    pan: DEFAULT_WORKFLOW_PAN,
  });
  const [panDraft, setPanDraft] = useState<PanDraft | null>(null);
  const { zoom, pan } = viewport;

  /** Converts viewport pointer coordinates into stable graph coordinates at any zoom. */
  function graphPoint(clientX: number, clientY: number): WorkflowPosition {
    const bounds = stageRef.current?.getBoundingClientRect();
    if (bounds === undefined) {
      return { x: 0, y: 0 };
    }
    return {
      x: (clientX - bounds.left) / zoom,
      y: (clientY - bounds.top) / zoom,
    };
  }

  /** Starts a connection preview and resolves the target beneath the pointer on release. */
  function startConnection(event: ReactPointerEvent, source: string): void {
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    setConnection({ source, pointer: graphPoint(event.clientX, event.clientY) });
  }

  /** Updates the temporary edge in graph space to avoid visual drift while zoomed. */
  function moveConnection(event: ReactPointerEvent): void {
    if (connection === null) {
      return;
    }
    setConnection({ ...connection, pointer: graphPoint(event.clientX, event.clientY) });
  }

  /** Connects only explicit input handles so accidental canvas releases remain harmless. */
  function finishConnection(event: ReactPointerEvent): void {
    if (connection === null) {
      return;
    }
    const targetElement = document
      .elementFromPoint(event.clientX, event.clientY)
      ?.closest<HTMLElement>("[data-workflow-input]");
    const target = targetElement?.dataset.workflowInput;
    if (target !== undefined && target !== connection.source) {
      onConnect(connection.source, target);
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
        "[data-workflow-node], [data-workflow-controls], button, input, textarea, [role=combobox]",
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

  /** Keeps canvas delete behavior scoped to the currently selected node. */
  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>): void {
    if ((event.key === "Delete" || event.key === "Backspace") && selectedNodeId !== null) {
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
      aria-label="工作流画布"
      tabIndex={0}
      onKeyDown={handleKeyDown}
      onWheel={handleWheel}
      onPointerDown={startPanning}
      onPointerMove={movePanning}
      onPointerUp={finishPanning}
      onPointerCancel={finishPanning}
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
        <WorkflowEdges nodes={nodes} edges={edges} connection={connection} />
        {nodes.map((node) => (
          <WorkflowNodeCard
            key={node.id}
            node={node}
            selected={selectedNodeId === node.id}
            zoom={zoom}
            onSelect={() => onSelectNode(node.id)}
            onMove={(position) => onMoveNode(node.id, position)}
            onStartConnection={(event) => startConnection(event, node.id)}
            onDelete={() => onDeleteNode(node.id)}
          />
        ))}
      </div>
      <div
        data-workflow-controls
        className="absolute bottom-3 left-3 z-30 flex w-fit items-center rounded-lg border border-border bg-background/95 p-1 shadow-sm backdrop-blur"
      >
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="缩小画布"
          disabled={zoom <= MIN_WORKFLOW_ZOOM}
          onClick={() => zoomFromCenter(zoom - 0.1)}
        >
          <IconMinus />
        </Button>
        <span className="w-12 text-center text-[11px] tabular-nums text-muted-foreground">
          {Math.round(zoom * 100)}%
        </span>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="放大画布"
          disabled={zoom >= MAX_WORKFLOW_ZOOM}
          onClick={() => zoomFromCenter(zoom + 0.1)}
        >
          <IconPlus />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="重置画布视图"
          onClick={() =>
            setViewport({
              zoom: DEFAULT_WORKFLOW_ZOOM,
              pan: DEFAULT_WORKFLOW_PAN,
            })
          }
        >
          <IconFocusCentered />
        </Button>
        <span className="hidden border-l border-border px-2 text-[10px] text-muted-foreground xl:inline">
          滚轮缩放 · 拖拽移动
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
}: {
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
  connection: ConnectionDraft | null;
}) {
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  return (
    <svg
      className="pointer-events-none absolute inset-0 z-0 overflow-visible"
      width={STAGE_WIDTH}
      height={STAGE_HEIGHT}
      aria-hidden="true"
    >
      <defs>
        <marker id="workflow-arrow" markerWidth="7" markerHeight="7" refX="6" refY="3.5" orient="auto">
          <path d="M0,0 L7,3.5 L0,7 Z" className="fill-muted-foreground/70" />
        </marker>
      </defs>
      {edges.map((edge) => {
        const source = nodeById.get(edge.source);
        const target = nodeById.get(edge.target);
        if (source === undefined || target === undefined) {
          return null;
        }
        const start = { x: source.position.x + NODE_WIDTH, y: source.position.y + NODE_ANCHOR_Y };
        const end = { x: target.position.x, y: target.position.y + NODE_ANCHOR_Y };
        return (
          <g key={edge.id}>
            <path
              d={edgePath(start, end)}
              fill="none"
              stroke="color-mix(in oklch, var(--foreground) 34%, transparent)"
              strokeWidth="2"
              markerEnd="url(#workflow-arrow)"
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
        const source = nodeById.get(connection.source);
        if (source === undefined) {
          return null;
        }
        return (
          <path
            d={edgePath(
              { x: source.position.x + NODE_WIDTH, y: source.position.y + NODE_ANCHOR_Y },
              connection.pointer,
            )}
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
  onDelete,
}: {
  node: WorkflowNode;
  selected: boolean;
  zoom: number;
  onSelect: () => void;
  onMove: (position: WorkflowPosition) => void;
  onStartConnection: (event: ReactPointerEvent) => void;
  onDelete: () => void;
}) {
  const metadata = getNodeMetadata(node.kind);
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
      className={cn(
        "absolute z-10 w-[230px] cursor-move rounded-xl border bg-card shadow-sm outline-none transition-[border-color,box-shadow] duration-200",
        selected
          ? "border-foreground/45 shadow-md ring-2 ring-ring/25"
          : "border-border hover:border-foreground/25 hover:shadow-md",
      )}
      style={{ left: node.position.x, top: node.position.y }}
      tabIndex={0}
      aria-label={`${metadata.label}节点：${node.title}`}
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
        aria-label={`连接到${node.title}`}
        className="absolute -left-3 top-[49px] flex size-6 items-center justify-center rounded-full outline-none after:size-3 after:rounded-full after:border-2 after:border-background after:bg-muted-foreground after:shadow-sm after:transition-transform hover:after:scale-125 focus-visible:ring-2 focus-visible:ring-ring"
      >
        <span className="sr-only">连接到{node.title}</span>
      </button>
      <div className="flex items-start gap-2.5 border-b border-border px-3 py-3">
        <span className={cn("flex size-8 shrink-0 items-center justify-center rounded-lg", metadata.tone)}>
          <Icon className="size-4" stroke={1.9} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <h4 className="truncate text-xs font-semibold">{node.title}</h4>
            <span className="rounded bg-muted px-1.5 py-0.5 text-[9px] font-medium text-muted-foreground">
              {metadata.label}
            </span>
          </div>
          <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-muted-foreground">
            {node.description}
          </p>
        </div>
        {selected && (
          <button
            type="button"
            aria-label={`删除${node.title}`}
            onClick={onDelete}
            className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-destructive/10 hover:text-destructive focus-visible:ring-2 focus-visible:ring-ring"
          >
            <IconTrash className="size-3.5" />
          </button>
        )}
      </div>
      <div className="flex items-center justify-between px-3 py-2 text-[10px] text-muted-foreground">
        <span className="truncate">{node.config.model ?? node.config.tool ?? "立即执行"}</span>
        <span className="font-mono text-[9px]">{node.id}</span>
      </div>
      <button
        type="button"
        aria-label={`从${node.title}开始连接`}
        onPointerDown={onStartConnection}
        className="absolute -right-3 top-[49px] flex size-6 items-center justify-center rounded-full outline-none after:size-3 after:rounded-full after:border-2 after:border-background after:bg-foreground after:shadow-sm after:transition-transform hover:after:scale-125 focus-visible:ring-2 focus-visible:ring-ring"
      >
        <span className="sr-only">从{node.title}开始连接</span>
      </button>
    </article>
  );
}
