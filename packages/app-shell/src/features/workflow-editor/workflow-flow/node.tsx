import { memo } from "react";
import {
  Handle,
  Position,
  useReactFlow,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import { useTranslation } from "react-i18next";
import { IconTrash } from "@tabler/icons-react";
import { cn } from "@ora/ui";
import {
  createMockWorkflowNodeType,
  WORKFLOW_NODE_ANCHOR_Y,
  WORKFLOW_NODE_WIDTH,
  type WorkflowNodeData,
} from "@ora/workflow-mock";
import {
  AgentExecutionModeMark,
  WorkflowNodeCardShell,
} from "../../workflow-node-chrome";
import { useWorkflowConnectionState } from "./use-connection-state";
import { WorkflowNodeParameterSummary } from "./node-parameter-summary";

/** Renders one workflow card with left/right handles styled for the definition editor. */
export const WorkflowFlowNodeView = memo(function WorkflowFlowNodeView({
  id,
  data,
  deletable,
  selected,
  positionAbsoluteX,
  positionAbsoluteY,
}: NodeProps<Node<WorkflowNodeData, "workflow">>) {
  const { i18n, t } = useTranslation();
  const { deleteElements } = useReactFlow<Node<WorkflowNodeData, "workflow">>();
  const { connectionCandidateEndpoint, connectionCandidateNodeId } =
    useWorkflowConnectionState();
  const locale =
    i18n.resolvedLanguage === "en-US" ? ("en-US" as const) : ("zh-CN" as const);
  const nodeKindLabel = createMockWorkflowNodeType(data.kind, locale).label;
  const isConnectionCandidate = connectionCandidateNodeId === id;
  const isInputCandidate =
    isConnectionCandidate && connectionCandidateEndpoint === "target";
  const isOutputCandidate =
    isConnectionCandidate && connectionCandidateEndpoint === "source";

  return (
    <WorkflowNodeCardShell
      data-workflow-node=""
      data-workflow-node-id={id}
      data-x={String(Math.round(positionAbsoluteX))}
      data-y={String(Math.round(positionAbsoluteY))}
      kind={data.kind}
      title={data.title}
      description={data.description}
      kindLabel={nodeKindLabel}
      density="editor"
      selected={selected}
      width={WORKFLOW_NODE_WIDTH}
      titleAccessory={
        data.kind === "agent" ? (
          <AgentExecutionModeMark
            interactive={data.agentConfig?.interactive === true}
          />
        ) : undefined
      }
      ariaLabel={`${t("settings.workflow.nodeSuffix", { type: nodeKindLabel })}: ${data.title}`}
      frameClassName={cn(
        isConnectionCandidate && "border-ring/60 shadow-md ring-2 ring-ring/10",
      )}
      details={<WorkflowNodeParameterSummary data={data} />}
      headerEnd={
        selected && deletable ? (
          <button
            type="button"
            className="nodrag nopan flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-destructive/10 hover:text-destructive focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={t("settings.workflow.deleteNamed", {
              name: data.title,
            })}
            onClick={() => {
              void deleteElements({ nodes: [{ id }] });
            }}
          >
            <IconTrash className="size-3.5" />
          </button>
        ) : undefined
      }
      targetHandle={
        <Handle
          type="target"
          position={Position.Left}
          data-workflow-input={id}
          aria-label={t("settings.workflow.connectTo", { name: data.title })}
          className={cn(
            "workflow-port workflow-port-input !size-2.5 !border-0 !bg-transparent",
            isInputCandidate && "workflow-port-candidate",
          )}
          style={{ top: WORKFLOW_NODE_ANCHOR_Y }}
        />
      }
      sourceHandle={
        <Handle
          type="source"
          position={Position.Right}
          data-workflow-output={id}
          aria-label={t("settings.workflow.connectFrom", { name: data.title })}
          className={cn(
            "workflow-port workflow-port-output !size-2.5 !border-0 !bg-transparent",
            isOutputCandidate && "workflow-port-candidate",
          )}
          style={{ top: WORKFLOW_NODE_ANCHOR_Y }}
        />
      }
    />
  );
});
