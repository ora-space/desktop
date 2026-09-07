import { Fragment, memo } from "react";
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
  createMockWorkflowCapabilities,
  createMockWorkflowNodeType,
  isWorkflowConditionComparisonComplete,
  resolveConditionCases,
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

const CONDITION_NODE_WIDTH = 320;
const CONDITION_FIRST_HANDLE_Y = 82;
const CONDITION_EMPTY_CASE_HEIGHT = 28;
const CONDITION_RULE_HEIGHT = 44;
const CONDITION_UNSET_RULE_HEIGHT = 28;
const CONDITION_LOGIC_CONNECTOR_HEIGHT = 14;

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
  const conditionCases =
    data.kind === "condition" ? resolveConditionCases(data) : [];

  return (
    <WorkflowNodeCardShell
      data-workflow-node=""
      data-workflow-node-id={id}
      data-x={String(Math.round(positionAbsoluteX))}
      data-y={String(Math.round(positionAbsoluteY))}
      kind={data.kind}
      title={data.title}
      description={data.description}
      kindLabel={id}
      density="editor"
      selected={selected}
      width={
        data.kind === "condition" ? CONDITION_NODE_WIDTH : WORKFLOW_NODE_WIDTH
      }
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
      details={
        data.kind === "condition" ? (
          <ConditionNodeDetails data={data} locale={locale} />
        ) : (
          <WorkflowNodeParameterSummary data={data} />
        )
      }
      detailsClassName={data.kind === "condition" ? "space-y-2" : undefined}
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
        data.kind === "condition" ? (
          <>
            {conditionCases.map((conditionCase, index) => (
              <Handle
                key={conditionCase.id}
                id={conditionCase.id}
                type="source"
                position={Position.Right}
                data-workflow-output={id}
                aria-label={`${t("settings.workflow.connectFrom", { name: data.title })} · ${conditionCase.id}`}
                className={cn(
                  "workflow-port workflow-port-output !size-2.5 !border-0 !bg-transparent",
                  isOutputCandidate && "workflow-port-candidate",
                )}
                style={{
                  top: conditionHandleTop(conditionCases, index),
                }}
              />
            ))}
            <Handle
              id="else"
              type="source"
              position={Position.Right}
              data-workflow-output={id}
              aria-label={`${t("settings.workflow.connectFrom", { name: data.title })} · else`}
              className={cn(
                "workflow-port workflow-port-output !size-2.5 !border-0 !bg-transparent",
                isOutputCandidate && "workflow-port-candidate",
              )}
              style={{
                top: conditionHandleTop(conditionCases, conditionCases.length),
              }}
            />
          </>
        ) : (
          <Handle
            type="source"
            position={Position.Right}
            data-workflow-output={id}
            aria-label={t("settings.workflow.connectFrom", {
              name: data.title,
            })}
            className={cn(
              "workflow-port workflow-port-output !size-2.5 !border-0 !bg-transparent",
              isOutputCandidate && "workflow-port-candidate",
            )}
            style={{ top: WORKFLOW_NODE_ANCHOR_Y }}
          />
        )
      }
    />
  );
});

/** Renders Dify-style IF / ELIF / ELSE rows whose labels align with branch handles. */
function ConditionNodeDetails({
  data,
  locale,
}: {
  data: WorkflowNodeData;
  locale: "zh-CN" | "en-US";
}) {
  const { t } = useTranslation();
  const cases = resolveConditionCases(data);
  const operators = createMockWorkflowCapabilities(locale).conditionOperators;
  return (
    <div className="space-y-2">
      {cases.map((conditionCase, caseIndex) =>
        conditionCase.conditions.length === 0 ? (
          <div
            key={conditionCase.id}
            className="flex h-5 items-center justify-end text-[10px] font-semibold"
          >
            {caseIndex === 0 ? "IF" : "ELIF"}
          </div>
        ) : (
          <div key={conditionCase.id} className="space-y-1">
            <div className="flex items-center justify-between text-[10px] font-semibold text-muted-foreground">
              <span>CASE {caseIndex + 1}</span>
              <span className="text-foreground">
                {caseIndex === 0 ? "IF" : "ELIF"}
              </span>
            </div>
            <div>
              {conditionCase.conditions.map((comparison, comparisonIndex) => (
                <Fragment key={comparisonIndex}>
                  {comparisonIndex > 0 && (
                    <div className="flex h-3.5 items-center justify-end pr-2 text-[10px] font-semibold text-blue-600 dark:text-blue-400">
                      {(conditionCase.logic ?? "and").toUpperCase()}
                    </div>
                  )}
                  <div className="min-w-0 rounded-lg bg-muted/70 px-2 py-1.5 text-[10px] leading-4">
                    {isWorkflowConditionComparisonComplete(comparison) ? (
                      <>
                        <div className="flex min-w-0 items-center gap-1 font-medium">
                          <span className="truncate text-muted-foreground">
                            {comparison.variableSelector[0]}
                          </span>
                          <span className="text-blue-600 dark:text-blue-400">
                            /
                          </span>
                          <span className="truncate text-blue-600 dark:text-blue-400">
                            {comparison.variableSelector.slice(1).join(".")}
                          </span>
                        </div>
                        <div className="flex min-w-0 items-center gap-2">
                          <span className="font-semibold">
                            {operators.find(
                              (operator) =>
                                operator.value === comparison.operator,
                            )?.label ?? comparison.operator}
                          </span>
                          <span className="truncate text-muted-foreground">
                            {conditionValueLabel(comparison.value)}
                          </span>
                        </div>
                      </>
                    ) : (
                      <span className="font-medium text-muted-foreground">
                        {t("settings.workflow.condition.unset")}
                      </span>
                    )}
                  </div>
                </Fragment>
              ))}
            </div>
          </div>
        ),
      )}
      <div className="flex justify-end pt-0.5 text-[10px] font-semibold">
        ELSE
      </div>
    </div>
  );
}

/** Aligns each branch handle with its rendered IF / ELIF / ELSE label. */
function conditionHandleTop(
  cases: ReturnType<typeof resolveConditionCases>,
  branchIndex: number,
): number {
  return cases
    .slice(0, branchIndex)
    .reduce(
      (top, conditionCase) =>
        top +
        (conditionCase.conditions.length === 0
          ? CONDITION_EMPTY_CASE_HEIGHT
          : CONDITION_EMPTY_CASE_HEIGHT +
            conditionCase.conditions.reduce(
              (height, comparison) =>
                height +
                (isWorkflowConditionComparisonComplete(comparison)
                  ? CONDITION_RULE_HEIGHT
                  : CONDITION_UNSET_RULE_HEIGHT),
              0,
            ) +
            Math.max(0, conditionCase.conditions.length - 1) *
              CONDITION_LOGIC_CONNECTOR_HEIGHT),
      CONDITION_FIRST_HANDLE_Y,
    );
}

/** Formats a condition comparison value for the compact canvas card. */
function conditionValueLabel(value: unknown): string {
  if (value === undefined || value === null) {
    return "";
  }
  return typeof value === "string" ? value : JSON.stringify(value);
}
