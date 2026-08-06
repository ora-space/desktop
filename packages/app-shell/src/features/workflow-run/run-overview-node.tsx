import { createContext, memo, useContext, type ReactNode } from "react";
import {
  Handle,
  Position,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import { useTranslation } from "react-i18next";
import { IconSparkles } from "@tabler/icons-react";
import { cn } from "@ora/ui";
import {
  createMockWorkflowNodeType,
  WORKFLOW_NODE_ANCHOR_Y,
  WORKFLOW_NODE_WIDTH,
} from "@ora/workflow-mock";
import { formatRunClock } from "../../lib/format";
import { WorkflowNodeCardShell } from "../workflow-node-chrome";
import { RunStatusBadge } from "./run-status-mark";
import { isNodeWorking, runStatusTone } from "./run-status-style";
import type {
  GraphWorkflowNodeState,
  WorkflowNodeData,
} from "@ora/workflow-runtime";
import "./theater-motion.css";

export type RunOverviewNodeData = WorkflowNodeData & {
  runStatus: GraphWorkflowNodeState["status"];
};

interface RunOverviewStatusMap {
  states: Record<string, GraphWorkflowNodeState>;
  focusedNodeId: string | null;
  activeNodeIds: string[];
  /** Soft affordance: artifact count per node id. */
  artifactCountByNode: Record<string, number>;
}

const RunOverviewStatusContext = createContext<RunOverviewStatusMap>({
  states: {},
  focusedNodeId: null,
  activeNodeIds: [],
  artifactCountByNode: {},
});

/** Provides live nodeStates to overview node renderers. */
export function RunOverviewStatusProvider({
  states,
  focusedNodeId,
  activeNodeIds,
  artifactCountByNode,
  children,
}: RunOverviewStatusMap & { children: ReactNode }) {
  return (
    <RunOverviewStatusContext.Provider
      value={{ states, focusedNodeId, activeNodeIds, artifactCountByNode }}
    >
      {children}
    </RunOverviewStatusContext.Provider>
  );
}

/**
 * Read-only run graph card on shared chrome + execution status overlay.
 */
export const RunOverviewNode = memo(function RunOverviewNode({
  id,
  data,
  selected,
}: NodeProps<Node<RunOverviewNodeData, "workflow">>) {
  const { i18n, t } = useTranslation();
  const { states, focusedNodeId, activeNodeIds, artifactCountByNode } = useContext(
    RunOverviewStatusContext,
  );
  const locale = i18n.resolvedLanguage === "en-US" ? "en-US" as const : "zh-CN" as const;
  const state = states[id] ?? { status: "idle" as const };
  const tone = runStatusTone(state.status);
  const kindLabel = createMockWorkflowNodeType(data.kind, locale).label;
  const focused = focusedNodeId === id || selected;
  const peerActive = !focused && activeNodeIds.includes(id);
  const artifactCount = artifactCountByNode[id] ?? 0;
  const startedLabel = state.startedAt !== undefined
    ? formatRunClock(state.startedAt, locale)
    : null;
  const finishedLabel = state.finishedAt !== undefined
    ? formatRunClock(state.finishedAt, locale)
    : null;
  const hasTiming = startedLabel !== null || finishedLabel !== null;
  const hasMetrics = hasTiming
    || state.durationMs !== undefined
    || state.tokenUsage?.totalTokens !== undefined;

  return (
    <WorkflowNodeCardShell
      data-workflow-run-node=""
      kind={data.kind}
      title={data.title}
      description={data.description}
      kindLabel={kindLabel}
      density="run"
      selected={focused}
      width={WORKFLOW_NODE_WIDTH * 0.92}
      ariaLabel={`${data.title}: ${t(tone.labelKey)}`}
      frameClassName={cn(
        tone.ring,
        "ring-1 transition-[box-shadow,ring-color] duration-300",
        state.status === "running" && "ring-sky-500/35 theater-live-breathe",
        state.status === "awaiting_input"
          && "ring-amber-500/35 theater-live-breathe-amber",
        peerActive && state.status !== "running" && state.status !== "awaiting_input"
          && "ring-sky-500/20",
      )}
      headerAccessory={(
        <div className="flex shrink-0 items-center gap-1">
          {artifactCount > 0 && (
            <span
              className="inline-flex items-center gap-0.5 rounded px-1 py-0.5 text-[9px] font-medium text-muted-foreground"
              title={t("workflowRun.artifacts.countBadge", {
                count: artifactCount,
              })}
            >
              <IconSparkles className="size-3" aria-hidden />
              <span className="tabular-nums">{artifactCount}</span>
            </span>
          )}
          <RunStatusBadge
            status={state.status}
            live={isNodeWorking(state.status)}
            className="px-1.5 py-0 text-[9px]"
          />
        </div>
      )}
      footer={hasMetrics
        ? (
          <p className="font-mono text-[9px] tabular-nums text-muted-foreground">
            {state.durationMs !== undefined ? `${state.durationMs}ms` : ""}
            {state.durationMs !== undefined
              && state.tokenUsage?.totalTokens !== undefined
              ? " · "
              : ""}
            {state.tokenUsage?.totalTokens !== undefined
              ? `${state.tokenUsage.totalTokens} tok`
              : ""}
            {hasTiming && (
              <span className="text-muted-foreground/55">
                {(state.durationMs !== undefined
                  || state.tokenUsage?.totalTokens !== undefined)
                  ? " · "
                  : ""}
                {startedLabel ?? "—"}
                {" — "}
                {finishedLabel ?? "—"}
              </span>
            )}
          </p>
        )
        : undefined}
      targetHandle={(
        <Handle
          type="target"
          position={Position.Left}
          className="!size-2 !border-0 !bg-transparent"
          style={{ top: WORKFLOW_NODE_ANCHOR_Y * 0.92 }}
          isConnectable={false}
        />
      )}
      sourceHandle={(
        <Handle
          type="source"
          position={Position.Right}
          className="!size-2 !border-0 !bg-transparent"
          style={{ top: WORKFLOW_NODE_ANCHOR_Y * 0.92 }}
          isConnectable={false}
        />
      )}
    />
  );
});
