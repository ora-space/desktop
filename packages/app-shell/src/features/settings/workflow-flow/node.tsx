import { memo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { useTranslation } from "react-i18next";
import { IconTrash } from "@tabler/icons-react";
import { cn } from "@ora/ui";
import {
  createMockWorkflowNodeType,
  type WorkflowLocale,
} from "@ora/workflow-mock";
import { getNodeMetadata } from "../workflow-node-metadata";
import { type WorkflowFlowNode } from "./adapters";
import {
  useWorkflowFlowActions,
  useWorkflowFlowConnectionState,
} from "./callbacks";

/** Renders one workflow card with left/right handles styled for the settings editor. */
export const WorkflowFlowNodeView = memo(function WorkflowFlowNodeView({
  id,
  data,
  selected,
  positionAbsoluteX,
  positionAbsoluteY,
}: NodeProps<WorkflowFlowNode>) {
  const { i18n, t } = useTranslation();
  const { onDeleteNode } = useWorkflowFlowActions();
  const {
    connectionCandidateEndpoint,
    connectionCandidateNodeId,
  } = useWorkflowFlowConnectionState();
  const locale: WorkflowLocale = i18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN";
  const metadata = getNodeMetadata(data.kind);
  const nodeKindLabel = createMockWorkflowNodeType(data.kind, locale).label;
  const Icon = metadata.icon;
  const detail = data.config.model
    ?? data.config.tool
    ?? t("settings.workflow.immediate");
  const isConnectionCandidate = connectionCandidateNodeId === id;
  const isInputCandidate = isConnectionCandidate
    && connectionCandidateEndpoint === "target";
  const isOutputCandidate = isConnectionCandidate
    && connectionCandidateEndpoint === "source";

  return (
    <article
      data-workflow-node
      data-workflow-node-id={id}
      data-x={String(Math.round(positionAbsoluteX))}
      data-y={String(Math.round(positionAbsoluteY))}
      className={cn(
        "group/workflow-node w-[230px] rounded-xl border bg-card shadow-sm outline-none transition-[border-color,box-shadow] duration-200",
        selected
          ? "border-foreground/45 shadow-md ring-2 ring-ring/25"
          : "border-border hover:border-foreground/25 hover:shadow-md",
        isConnectionCandidate && "border-ring/60 shadow-md ring-2 ring-ring/10",
      )}
      aria-label={`${t("settings.workflow.nodeSuffix", { type: nodeKindLabel })}: ${data.title}`}
    >
      <Handle
        type="target"
        position={Position.Left}
        data-workflow-input={id}
        aria-label={t("settings.workflow.connectTo", { name: data.title })}
        className={cn(
          "workflow-port workflow-port-input !size-6 !border-0 !bg-transparent",
          isInputCandidate && "workflow-port-candidate",
        )}
        style={{ top: 61 }}
      />
      <div className="flex items-start gap-2.5 border-b border-border px-3 py-3">
        <span className={cn("flex size-8 shrink-0 items-center justify-center rounded-lg", metadata.tone)}>
          <Icon className="size-4" stroke={1.9} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <h4 className="truncate text-xs font-semibold">{data.title}</h4>
            <span className="rounded bg-muted px-1.5 py-0.5 text-[9px] font-medium text-muted-foreground">
              {nodeKindLabel}
            </span>
          </div>
          <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-muted-foreground">
            {data.description}
          </p>
        </div>
        {selected && (
          <button
            type="button"
            className="nodrag nopan flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-destructive/10 hover:text-destructive focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={t("settings.workflow.deleteNamed", { name: data.title })}
            onClick={() => onDeleteNode(id)}
          >
            <IconTrash className="size-3.5" />
          </button>
        )}
      </div>
      <div className="flex items-center justify-between px-3 py-2 text-[10px] text-muted-foreground">
        <span className="truncate">{detail}</span>
        <span className="font-mono text-[9px]">{id}</span>
      </div>
      <Handle
        type="source"
        position={Position.Right}
        data-workflow-output={id}
        aria-label={t("settings.workflow.connectFrom", { name: data.title })}
        className={cn(
          "workflow-port workflow-port-output !size-6 !border-0 !bg-transparent",
          isOutputCandidate && "workflow-port-candidate",
        )}
        style={{ top: 61 }}
      />
    </article>
  );
});
