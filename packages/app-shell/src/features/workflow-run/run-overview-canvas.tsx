import {
  useEffect,
  useMemo,
  useRef,
  type MutableRefObject,
  type RefObject,
} from "react";
import { useTranslation } from "react-i18next";
import {
  Background,
  BackgroundVariant,
  MarkerType,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type DefaultEdgeOptions,
  type Edge,
  type Node,
} from "@xyflow/react";
import {
  MAX_WORKFLOW_ZOOM,
  MIN_WORKFLOW_ZOOM,
} from "../workflow-editor/workflow-flow/viewport";
import { resolveOverviewFocusedId, resolveTheaterFocus } from "./run-focus";
import {
  RunOverviewNode,
  RunOverviewStatusProvider,
  type RunOverviewNodeData,
} from "./run-overview-node";
import { RunOverviewEdge } from "./run-overview-edge";
import type { GraphWorkflowRun, WorkflowArtifact } from "@ora/workflow-runtime";
import "@xyflow/react/dist/style.css";

const NODE_TYPE = "workflow" as const;
const EDGE_TYPE = "workflow" as const;
const FIT_PADDING = 0.18;
const RESIZE_FIT_DEBOUNCE_MS = 160;

const nodeTypes = { [NODE_TYPE]: RunOverviewNode };
const edgeTypes = { [EDGE_TYPE]: RunOverviewEdge };

const DEFAULT_EDGE_OPTIONS = {
  type: EDGE_TYPE,
  selectable: false,
  focusable: false,
  markerEnd: {
    type: MarkerType.ArrowClosed,
    width: 22,
    height: 22,
    markerUnits: "userSpaceOnUse",
    color: "color-mix(in oklch, var(--foreground) 40%, transparent)",
  },
} satisfies DefaultEdgeOptions;

interface RunOverviewCanvasProps {
  run: GraphWorkflowRun;
  focusedNodeId: string | null;
  onFocusNode: (nodeId: string) => void;
  /** Used for a soft per-node artifact affordance (count only). */
  artifacts?: WorkflowArtifact[];
  /**
   * Bump to re-run fitView and re-enable resize auto-fit (e.g. user clicks
   * Overview again after a manual pan/zoom).
   */
  fitRequestKey?: number;
}

/**
 * Fits on demand and on container resize until the user pan/zooms the graph.
 * Explicit fitRequest / snapshot change clears the manual lock.
 * Mode-enter and resize fits are instant — animated fitView after remount
 * reads as a top-to-bottom jump when switching from Theater.
 */
function OverviewViewportController({
  containerRef,
  snapshotId,
  fitRequestKey,
  userAdjustedRef,
}: {
  containerRef: RefObject<HTMLDivElement | null>;
  snapshotId: string;
  fitRequestKey: number;
  userAdjustedRef: MutableRefObject<boolean>;
}) {
  const { fitView } = useReactFlow();
  const suppressResizeFitRef = useRef(true);

  useEffect(() => {
    userAdjustedRef.current = false;
    // Ignore ResizeObserver callbacks that fire from the Theater→Overview
    // layout swap; those would otherwise queue a second fit right after mount.
    suppressResizeFitRef.current = true;
    const frame = requestAnimationFrame(() => {
      void fitView({ padding: FIT_PADDING, duration: 0 });
      // Allow resize auto-fit only after the enter fit has committed.
      requestAnimationFrame(() => {
        suppressResizeFitRef.current = false;
      });
    });
    return () => cancelAnimationFrame(frame);
  }, [fitRequestKey, fitView, snapshotId, userAdjustedRef]);

  useEffect(() => {
    const container = containerRef.current;
    if (container === null || typeof ResizeObserver === "undefined") {
      return;
    }
    let timer: ReturnType<typeof setTimeout> | null = null;
    const observer = new ResizeObserver(() => {
      if (userAdjustedRef.current || suppressResizeFitRef.current) {
        return;
      }
      if (timer !== null) {
        clearTimeout(timer);
      }
      timer = setTimeout(() => {
        timer = null;
        if (!userAdjustedRef.current && !suppressResizeFitRef.current) {
          void fitView({ padding: FIT_PADDING, duration: 0 });
        }
      }, RESIZE_FIT_DEBOUNCE_MS);
    });
    observer.observe(container);
    return () => {
      observer.disconnect();
      if (timer !== null) {
        clearTimeout(timer);
      }
    };
  }, [containerRef, fitView, userAdjustedRef]);

  return null;
}

/**
 * Read-only React Flow overview of a frozen run snapshot + live nodeStates.
 * Clicking a node focuses it for Theater (caller switches mode).
 */
export function RunOverviewCanvas({
  run,
  focusedNodeId,
  onFocusNode,
  artifacts = [],
  fitRequestKey = 0,
}: RunOverviewCanvasProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const userAdjustedRef = useRef(false);
  const snapshot = run.definitionSnapshot;
  const nodeStates = run.nodeStates;
  const focus = useMemo(
    () => resolveTheaterFocus(run, focusedNodeId),
    [run, focusedNodeId],
  );
  // Terminal + no pin: do not paint Theater's fallback as selected —
  // Theater shows the result act for the same state.
  const overviewFocusedId = useMemo(
    () => resolveOverviewFocusedId(run, focusedNodeId),
    [run, focusedNodeId],
  );
  const artifactCountByNode = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const artifact of artifacts) {
      counts[artifact.nodeId] = (counts[artifact.nodeId] ?? 0) + 1;
    }
    return counts;
  }, [artifacts]);

  const nodes = useMemo((): Node<RunOverviewNodeData, "workflow">[] => {
    return snapshot.nodes.map((node) => ({
      ...node,
      type: NODE_TYPE,
      selectable: true,
      draggable: false,
      connectable: false,
      deletable: false,
      data: {
        ...node.data,
        runStatus: nodeStates[node.id]?.status ?? "idle",
      },
    }));
  }, [snapshot.nodes, nodeStates]);

  const edges = useMemo((): Edge[] => {
    return snapshot.edges.map((edge) => {
      const sourceStatus = nodeStates[edge.source]?.status ?? "idle";
      const activePath = sourceStatus !== "idle";
      return {
        ...edge,
        type: EDGE_TYPE,
        selectable: false,
        focusable: false,
        reconnectable: false,
        data: { ...(edge.data ?? {}), activePath },
      };
    });
  }, [snapshot.edges, nodeStates]);

  return (
    <div
      ref={containerRef}
      className="relative min-h-0 flex-1 bg-muted/15"
      aria-label={t("workflowRun.overview.label")}
    >
      <ReactFlowProvider>
        <RunOverviewStatusProvider
          states={nodeStates}
          focusedNodeId={overviewFocusedId}
          activeNodeIds={focus.activeIds}
          artifactCountByNode={artifactCountByNode}
        >
          <ReactFlow
            nodes={nodes}
            edges={edges}
            nodeTypes={nodeTypes}
            edgeTypes={edgeTypes}
            defaultEdgeOptions={DEFAULT_EDGE_OPTIONS}
            nodesDraggable={false}
            nodesConnectable={false}
            elementsSelectable
            edgesReconnectable={false}
            panOnScroll
            zoomOnScroll
            minZoom={MIN_WORKFLOW_ZOOM}
            maxZoom={MAX_WORKFLOW_ZOOM}
            proOptions={{ hideAttribution: true }}
            onMoveEnd={(event) => {
              // Programmatic fitView reports a null event; user gestures do not.
              if (event !== null) {
                userAdjustedRef.current = true;
              }
            }}
            onNodeClick={(_event, node) => {
              onFocusNode(node.id);
            }}
            className="h-full w-full"
          >
            <OverviewViewportController
              containerRef={containerRef}
              snapshotId={snapshot.id}
              fitRequestKey={fitRequestKey}
              userAdjustedRef={userAdjustedRef}
            />
            <Background
              id="run-overview-dots"
              variant={BackgroundVariant.Dots}
              gap={22}
              size={1.1}
              color="color-mix(in oklch, var(--foreground) 12%, transparent)"
            />
          </ReactFlow>
        </RunOverviewStatusProvider>
      </ReactFlowProvider>
      <p className="pointer-events-none absolute bottom-3 left-3 rounded-md border border-border/70 bg-background/85 px-2 py-1 text-[10px] text-muted-foreground backdrop-blur-sm">
        {t("workflowRun.overview.hint")}
      </p>
    </div>
  );
}
