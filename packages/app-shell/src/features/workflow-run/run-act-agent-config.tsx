import { useTranslation } from "react-i18next";
import { cn } from "@ora/ui";
import type { WorkflowAgentConfig } from "@ora/workflow-runtime";
import { IconRobot } from "@tabler/icons-react";
import { PluginLogoMark } from "../settings/plugin-logo";
import { useAgentCatalog } from "../chat/agent-catalog";
import { useAgents } from "../../state/hooks/use-agents";
import { useSkills } from "../../state/hooks/use-skills";
import { formatAgentExecutorLabel } from "./agent-config-display";
import { RunBriefPopover } from "./run-brief-popover";
import { shouldPreviewBrief } from "./should-preview-brief";

interface RunActAgentConfigProps {
  config: WorkflowAgentConfig;
}

/**
 * Read-only Agent contract for the run inspector — settings field parity without
 * editable controls. Role, enabled skills, and enabled MCP bindings each open a
 * brief popover (catalog label/description, or a quiet “no description” tip when
 * empty). Long prompt text also opens a preview when it would otherwise truncate.
 */
export function RunActAgentConfig({ config }: RunActAgentConfigProps) {
  const { t } = useTranslation();
  const agentCatalog = useAgentCatalog();
  const agentsQuery = useAgents();
  const skillsQuery = useSkills();
  // The workflow JSON stores role/skill by name, so resolve catalog descriptions by name.
  const agentByName = new Map(
    (agentsQuery.data ?? []).map((agent) => [agent.name, agent]),
  );
  const skillByName = new Map(
    (skillsQuery.data ?? []).map((skill) => [skill.name, skill]),
  );
  const role = agentByName.get(config.roleId);
  const roleLabel = role?.name ?? config.roleId;
  const roleDescription = role?.description?.trim() ?? "";
  const modelLabel = formatAgentExecutorLabel(config.executor, agentCatalog);
  const enabledSkills = config.skills.filter((skill) => skill.enabled);
  const enabledMcps = (config.mcps ?? []).filter((mcp) => mcp.enabled);
  const agentLogo = agentCatalog.find(
    (agent) => agent.agentRef === config.executor.agentCli,
  )?.logo;
  const prompt = config.prompt.trim();

  return (
    <>
      <div className="space-y-1">
        <p className="text-[11px] text-muted-foreground">
          {t("settings.workflow.field.agentModel")}
        </p>
        <div
          data-selectable
          className="flex min-w-0 items-center gap-2 rounded-lg border border-border/70 bg-muted/25 px-3 py-2"
        >
          <PluginLogoMark
            logo={agentLogo}
            fallback={IconRobot}
            className="size-3.5 shrink-0 object-contain"
          />
          <span className="min-w-0 truncate font-mono text-[11px] text-foreground/90">
            {modelLabel}
          </span>
        </div>
      </div>

      <div className="space-y-1">
        <p className="text-[11px] text-muted-foreground">
          {t("settings.workflow.field.role")}
        </p>
        <RunBriefPopover
          title={roleLabel}
          body={
            roleDescription === ""
              ? t("workflowRun.inspector.catalogNoDescription")
              : roleDescription
          }
          openLabel={t("workflowRun.inspector.roleOpen", { name: roleLabel })}
        >
          <span className="line-clamp-2 text-xs leading-4">{roleLabel}</span>
        </RunBriefPopover>
      </div>

      {enabledSkills.length > 0 && (
        <div className="space-y-1">
          <p className="text-[11px] text-muted-foreground">
            {t("settings.workflow.field.skills")}
          </p>
          <ul
            className="space-y-1.5"
            aria-label={t("settings.workflow.field.skills")}
          >
            {enabledSkills.map((binding) => {
              const skill = skillByName.get(binding.skillId);
              const name = skill?.name ?? binding.skillId;
              const description = skill?.description?.trim() ?? "";
              return (
                <li key={binding.skillId}>
                  <RunBriefPopover
                    title={name}
                    body={
                      description === ""
                        ? t("workflowRun.inspector.catalogNoDescription")
                        : description
                    }
                    openLabel={t("workflowRun.inspector.skillOpen", { name })}
                  >
                    <span className="line-clamp-2 text-xs leading-4">
                      {name}
                    </span>
                  </RunBriefPopover>
                </li>
              );
            })}
          </ul>
        </div>
      )}

      {enabledMcps.length > 0 && (
        <div className="space-y-1">
          <p className="text-[11px] text-muted-foreground">
            {t("settings.workflow.field.mcps")}
          </p>
          <ul
            className="space-y-1.5"
            aria-label={t("settings.workflow.field.mcps")}
          >
            {enabledMcps.map((mcp) => (
              <li key={mcp.mcpId}>
                <RunBriefPopover
                  title={mcp.mcpId}
                  body={t("workflowRun.inspector.catalogNoDescription")}
                  openLabel={t("workflowRun.inspector.mcpOpen", {
                    name: mcp.mcpId,
                  })}
                >
                  <span className="line-clamp-2 text-xs leading-4">
                    {mcp.mcpId}
                  </span>
                </RunBriefPopover>
              </li>
            ))}
          </ul>
        </div>
      )}

      <div className="space-y-1">
        <p className="text-[11px] text-muted-foreground">
          {t("settings.workflow.field.prompt")}
        </p>
        {shouldPreviewBrief(prompt) ? (
          <RunBriefPopover
            title={t("settings.workflow.field.prompt")}
            body={prompt}
            openLabel={t("workflowRun.inspector.textOpen", {
              field: t("settings.workflow.field.prompt"),
            })}
          >
            <span className="line-clamp-4 whitespace-pre-wrap text-xs leading-5">
              {prompt}
            </span>
          </RunBriefPopover>
        ) : (
          <StaticValue value={prompt} multiline />
        )}
      </div>
    </>
  );
}

/** Quiet static field chrome when there is no brief to preview. */
function StaticValue({
  value,
  multiline = false,
}: {
  value: string;
  multiline?: boolean;
}) {
  return (
    <div
      data-selectable
      className={cn(
        "rounded-lg border border-border/70 bg-muted/25 px-3 py-2 text-xs text-foreground/90",
        multiline && "max-h-40 overflow-y-auto whitespace-pre-wrap leading-5",
      )}
    >
      {value === "" ? "—" : value}
    </div>
  );
}
