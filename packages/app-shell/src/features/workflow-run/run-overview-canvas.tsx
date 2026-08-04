import { useEffect, useMemo } from "react";
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
} from "../settings/workflow-flow/viewport";
import { resolveTheaterFocus } from "./run-focus";
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
   * Bump to re-run fitView while the canvas stays mounted (e.g. user clicks
   * Overview again after resizing the pane).
   */
  fitRequestKey?: number;
}

/** Fits the read-only graph after mount, snapshot change, or an explicit refit. */
function FitViewOnRequest({
  snapshotId,
  fitRequestKey,
}: {
  snapshotId: string;
  fitRequestKey: number;
}) {
  const { fitView } = useReactFlow();
  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      void fitView({ padding: 0.18, duration: 200 });
    });
    return () => cancelAnimationFrame(frame);
  }, [fitView, snapshotId, fitRequestKey]);
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
  const snapshot = run.definitionSnapshot;
  const nodeStates = run.nodeStates;
  const focus = useMemo(
    () => resolveTheaterFocus(run, focusedNodeId),
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
      const targetStatus = nodeStates[edge.target]?.status ?? "idle";
      const activePath =
        sourceStatus !== "skipped"
        && sourceStatus !== "idle"
        && targetStatus !== "skipped";
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
      className="relative min-h-0 flex-1 bg-muted/15"
      aria-label={t("workflowRun.overview.label")}
    >
      <ReactFlowProvider>
        <RunOverviewStatusProvider
          states={nodeStates}
          focusedNodeId={focus.primaryId}
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
            fitView
            proOptions={{ hideAttribution: true }}
            onNodeClick={(_event, node) => {
              onFocusNode(node.id);
            }}
            className="h-full w-full"
          >
            <FitViewOnRequest
              snapshotId={snapshot.id}
              fitRequestKey={fitRequestKey}
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
