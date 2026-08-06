import { useTranslation } from "react-i18next";
import { type WorkflowNodeData } from "@ora/workflow-mock";
import { useAgents } from "../../../state/hooks/use-agents";
import { useSkills } from "../../../state/hooks/use-skills";

interface NodeParameter {
  label: string;
  values: string[];
}

/** Displays the persisted node configuration without introducing card-level editing controls. */
export function WorkflowNodeParameterSummary({ data }: { data: WorkflowNodeData }) {
  const { t } = useTranslation();
  const agentsQuery = useAgents();
  const skillsQuery = useSkills();
  const agentNameById = new Map((agentsQuery.data ?? []).map((agent) => [agent.id, agent.name]));
  const skillNameById = new Map((skillsQuery.data ?? []).map((skill) => [skill.id, skill.name]));
  const parameters = configuredParameters(data, t, agentNameById, skillNameById);

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
  t: (key: string) => string,
  agentNameById: ReadonlyMap<string, string>,
  skillNameById: ReadonlyMap<string, string>,
): NodeParameter[] {
  const parameters: NodeParameter[] = [];
  if (data.kind === "agent" && data.agentConfig !== undefined) {
    const enabledSkills = data.agentConfig.skills
      .filter((skill) => skill.enabled)
      .map((skill) => skillNameById.get(skill.skillId) ?? skill.skillId);
    parameters.push(
      {
        label: t("settings.workflow.field.role"),
        values: [agentNameById.get(data.agentConfig.roleId) ?? data.agentConfig.roleId],
      },
      {
        label: t("settings.workflow.field.agentModel"),
        values: [`${data.agentConfig.executor.agentCli} · ${data.agentConfig.executor.modelId}`],
      },
    );
    if (enabledSkills.length > 0) {
      parameters.push({
        label: t("settings.workflow.field.skills"),
        values: enabledSkills.slice(0, 3),
      });
    }
    return parameters;
  }

  appendParameter(parameters, t("settings.workflow.field.model"), data.model);
  appendParameter(parameters, t("settings.workflow.field.tool"), data.tool);
  appendParameter(parameters, t("settings.workflow.field.condition"), data.condition);
  appendParameter(parameters, t("settings.workflow.field.instruction"), data.instruction);
  return parameters;
}

/** Keeps absent and whitespace-only values out of the summary so empty defaults do not add visual noise. */
function appendParameter(
  parameters: NodeParameter[],
  label: string,
  value: string | undefined,
): void {
  const trimmedValue = value?.trim();
  if (trimmedValue !== undefined && trimmedValue !== "") {
    parameters.push({ label, values: [trimmedValue] });
  }
}
