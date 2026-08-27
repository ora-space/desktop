import { useTranslation } from "react-i18next";
import { type WorkflowNodeData } from "@ora/workflow-mock";
import {
  conditionBranchesSummary,
  createWorkflowSummaryLabels,
  junctionFailureStrategyLabel,
  junctionWaitStrategyLabel,
  type WorkflowSummaryLabels,
} from "../../workflow-node-chrome";
import { useAgents } from "../../../state/hooks/use-agents";
import { useSkills } from "../../../state/hooks/use-skills";
import { MCP_CATALOG } from "../mcp-catalog";

interface NodeParameter {
  label: string;
  values: string[];
}

/** Displays the persisted node configuration without introducing card-level editing controls. */
export function WorkflowNodeParameterSummary({
  data,
}: {
  data: WorkflowNodeData;
}) {
  const { i18n, t } = useTranslation();
  const agentsQuery = useAgents();
  const skillsQuery = useSkills();
  // The workflow JSON stores role/skill by name, so name-keyed lookups resolve directly.
  const agentNameById = new Map(
    (agentsQuery.data ?? []).map((agent) => [agent.name, agent.name]),
  );
  const skillNameById = new Map(
    (skillsQuery.data ?? []).map((skill) => [skill.name, skill.name]),
  );
  const mcpNameById = new Map(MCP_CATALOG.map((mcp) => [mcp.id, mcp.name]));
  const locale =
    i18n.resolvedLanguage === "en-US" ? ("en-US" as const) : ("zh-CN" as const);
  const labels = createWorkflowSummaryLabels(locale);
  const parameters = configuredParameters(
    data,
    t,
    agentNameById,
    skillNameById,
    mcpNameById,
    labels,
    locale,
  );

  if (parameters.length === 0) {
    return null;
  }

  return (
    <dl
      aria-label={t("settings.workflow.nodeParameters")}
      className="space-y-2"
    >
      {parameters.map((parameter) => (
        <div key={parameter.label} className="min-w-0">
          <dt className="mb-1 text-[9px] font-medium text-muted-foreground">
            {parameter.label}
          </dt>
          <div className="space-y-1">
            {parameter.values.map((value) => (
              <dd
                key={`${parameter.label}:${value}`}
                className="m-0 line-clamp-2 break-words rounded-md bg-muted px-2 py-1 text-[10px] leading-4 text-foreground/85 shadow-inner"
              >
                {value}
              </dd>
            ))}
          </div>
        </div>
      ))}
    </dl>
  );
}

/** Extracts only populated execution fields so cards summarize the saved configuration compactly. */
function configuredParameters(
  data: WorkflowNodeData,
  t: (key: string, options?: Record<string, unknown>) => string,
  agentNameById: ReadonlyMap<string, string>,
  skillNameById: ReadonlyMap<string, string>,
  mcpNameById: ReadonlyMap<string, string>,
  labels: WorkflowSummaryLabels,
  locale: "zh-CN" | "en-US",
): NodeParameter[] {
  const parameters: NodeParameter[] = [];
  if (data.kind === "agent" && data.agentConfig !== undefined) {
    const enabledSkills = (data.agentConfig.skills ?? [])
      .filter((skill) => skill.enabled)
      .map((skill) => skillNameById.get(skill.skillId) ?? skill.skillId);
    const enabledMcps = (data.agentConfig.mcps ?? [])
      .filter((mcp) => mcp.enabled)
      .map((mcp) => mcpNameById.get(mcp.mcpId) ?? mcp.mcpId);
    parameters.push(
      {
        label: t("settings.workflow.field.role"),
        values: [
          agentNameById.get(data.agentConfig.roleId) ?? data.agentConfig.roleId,
        ],
      },
      {
        label: t("settings.workflow.field.agentModel"),
        values: [
          `${data.agentConfig.executor.agentCli} · ${data.agentConfig.executor.modelId}`,
        ],
      },
    );
    if (enabledSkills.length > 0) {
      parameters.push({
        label: t("settings.workflow.field.skills"),
        values: enabledSkills.slice(0, 3),
      });
    }
    if (enabledMcps.length > 0) {
      parameters.push({
        label: t("settings.workflow.field.mcps"),
        values: enabledMcps.slice(0, 3),
      });
    }
    return parameters;
  }
  if (data.kind === "condition") {
    appendParameter(
      parameters,
      t("settings.workflow.field.condition"),
      conditionBranchesSummary(data, labels, locale) ?? undefined,
    );
    appendParameter(
      parameters,
      t("settings.workflow.field.instruction"),
      data.instruction,
    );
    return parameters;
  }
  if (data.kind === "tool") {
    appendParameter(parameters, t("settings.workflow.field.tool"), data.tool);
    if (data.operation !== undefined && data.operation !== "") {
      appendParameter(
        parameters,
        t("settings.workflow.field.operation"),
        labels.operationLabel(data.operation),
      );
    }
    if ((data.toolParameters ?? []).length > 0) {
      appendParameter(
        parameters,
        t("settings.workflow.section.parameters"),
        data.toolParameters!.map(
          (parameter) => `${parameter.key} = ${parameter.value}`,
        ),
      );
    }
    appendParameter(
      parameters,
      t("settings.workflow.field.instruction"),
      data.instruction,
    );
    return parameters;
  }

  if (data.kind === "start") {
    appendParameter(
      parameters,
      t("settings.workflow.field.trigger"),
      data.trigger === undefined
        ? undefined
        : labels.triggerLabel(data.trigger),
    );
    appendParameter(
      parameters,
      t("settings.workflow.field.instruction"),
      data.instruction,
    );
    return parameters;
  }
  if (data.kind === "junction") {
    appendParameter(
      parameters,
      t("settings.workflow.field.waitStrategy"),
      junctionWaitStrategyLabel(data.waitStrategy, t),
    );
    appendParameter(
      parameters,
      t("settings.workflow.field.failureStrategy"),
      junctionFailureStrategyLabel(data.failureStrategy, t),
    );
    appendParameter(
      parameters,
      t("settings.workflow.field.instruction"),
      data.instruction,
    );
    return parameters;
  }
  if (data.kind === "loop") {
    appendParameter(
      parameters,
      t("settings.workflow.field.maxAttempts"),
      data.maxAttempts?.toString(),
    );
    appendParameter(
      parameters,
      t("settings.workflow.field.exitCondition"),
      data.exitCondition,
    );
    appendParameter(
      parameters,
      t("settings.workflow.field.instruction"),
      data.instruction,
    );
    return parameters;
  }

  appendParameter(
    parameters,
    t("settings.workflow.field.instruction"),
    data.instruction,
  );
  return parameters;
}

/** Keeps absent and whitespace-only values out of the summary so empty defaults do not add visual noise. */
function appendParameter(
  parameters: NodeParameter[],
  label: string,
  value: string | string[] | undefined,
): void {
  const values = Array.isArray(value) ? value : [value];
  const populated = values
    .map((entry) => entry?.trim())
    .filter((entry): entry is string => entry !== undefined && entry !== "");
  if (populated.length > 0) {
    parameters.push({ label, values: populated });
  }
}
