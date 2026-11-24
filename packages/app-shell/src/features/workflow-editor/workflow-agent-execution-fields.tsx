import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconRobot,
  IconLoader2,
  IconChevronDown,
  IconCheck,
} from "@tabler/icons-react";
import {
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@ora/ui";
import type {
  WorkflowVariableCatalogEntry,
  WorkflowAgentModel,
  WorkflowAgentConfig,
  WorkflowCapabilities,
} from "@ora/workflow-mock";
import { PluginLogoMark } from "../settings/plugin-logo";
import {
  agentLabel,
  type AgentEntry,
} from "../../state/hooks/use-agent-catalog";
import type { WorkflowAgentCliStatus } from "../../state/hooks/use-workflow-agent-models";
import { WorkflowPromptEditor } from "./workflow-prompt-editor";
import { InspectorField } from "./workflow-node-details";

/**
 * Catalog labels are stored as `agent · model` for the flat picker; the
 * two-section menu shows the model name alone, matching chat.
 */
function workflowModelDisplayName(
  model: WorkflowAgentModel,
  agents: readonly AgentEntry[],
): string {
  const prefix = `${agentLabel(agents, model.agentCli)} · `;
  return model.label.startsWith(prefix)
    ? model.label.slice(prefix.length)
    : model.label;
}

export function AgentExecutionFields({
  config,
  capabilities,
  agents,
  modelsByCli,
  cliStatus,
  modelsLoading = false,
  modelsError,
  onRetryModels,
  variableCatalog,
  onChange,
}: {
  config: Pick<WorkflowAgentConfig, "executor" | "prompt">;
  capabilities: Pick<WorkflowCapabilities, "agentModels">;
  agents?: AgentEntry[];
  modelsByCli?: ReadonlyMap<string, WorkflowAgentModel[]>;
  cliStatus?: Readonly<Record<string, WorkflowAgentCliStatus>>;
  modelsLoading?: boolean;
  modelsError?: boolean;
  onRetryModels?: () => void;
  variableCatalog: WorkflowVariableCatalogEntry[];
  onChange: (
    patch: Partial<Pick<WorkflowAgentConfig, "executor" | "prompt">>,
  ) => void;
}) {
  const { t } = useTranslation();
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const offeredAgents = agents ?? [];
  const currentAgentCli = config.executor.agentCli;
  const configuredModel = capabilities.agentModels.find(
    (model) =>
      model.agentCli === config.executor.agentCli &&
      model.modelId === config.executor.modelId,
  );
  const selectedModel = configuredModel ?? {
    agentCli: config.executor.agentCli,
    modelId: config.executor.modelId,
    label: `${agentLabel(offeredAgents, config.executor.agentCli)} · ${config.executor.modelId}`,
  };
  const modelsForSelectedCli =
    modelsByCli?.get(currentAgentCli) ??
    capabilities.agentModels.filter(
      (model) => model.agentCli === currentAgentCli,
    );
  const selectedCliStatus = cliStatus?.[currentAgentCli];
  // Model discovery is per-CLI: the selected CLI is loading, so the model
  // group below is still on its way rather than genuinely empty.
  const selectedCliLoading =
    modelsLoading || selectedCliStatus?.isLoading === true;
  // A node always shows its model name; when the executor is not backed by a
  // discovered model (e.g. a CLI that failed to report one) the full
  // `agent · model` pair is shown instead so the agent pick stays legible.
  const selectedModelName =
    configuredModel === undefined
      ? selectedModel.label
      : workflowModelDisplayName(selectedModel, offeredAgents);
  /**
   * Switches the node onto another agent. Keeps the current model id when that
   * agent offers it; otherwise falls back to the first discovered model so the
   * executor pair stays catalog-backed. An agent with no discovered models keeps
   * the current id rather than inventing one — the model group then shows the
   * empty state and the pick stays visible (never reverted).
   */
  function selectAgentCli(agentCli: string): void {
    if (agentCli === config.executor.agentCli) {
      return;
    }
    const models =
      modelsByCli?.get(agentCli) ??
      capabilities.agentModels.filter((model) => model.agentCli === agentCli);
    const kept = models.find(
      (model) => model.modelId === config.executor.modelId,
    );
    onChange({
      executor: {
        agentCli,
        modelId: kept?.modelId ?? models[0]?.modelId ?? config.executor.modelId,
      },
    });
  }

  return (
    <>
      <InspectorField
        label={t("settings.workflow.field.agentModel")}
        htmlFor="workflow-agent-model"
      >
        <Popover open={modelPickerOpen} onOpenChange={setModelPickerOpen}>
          <PopoverTrigger
            render={
              <Button
                id="workflow-agent-model"
                type="button"
                variant="outline"
                className="h-9 w-full min-w-0 shrink justify-between overflow-hidden px-3 font-normal"
                disabled={
                  capabilities.agentModels.length === 0 && !selectedCliLoading
                }
                aria-label={t("settings.workflow.field.agentModel")}
              />
            }
          >
            <span className="flex w-full min-w-0 items-center justify-between gap-2">
              <span className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden text-left">
                <PluginLogoMark
                  logo={
                    offeredAgents.find(
                      (agent) => agent.agentRef === currentAgentCli,
                    )?.logo
                  }
                  fallback={IconRobot}
                  className="size-3.5 shrink-0 object-contain"
                />
                <span className="min-w-0 truncate">{selectedModelName}</span>
              </span>
              {selectedCliLoading ? (
                <IconLoader2
                  data-testid="workflow-agent-model-loading"
                  className="size-3.5 shrink-0 animate-spin opacity-50"
                  aria-hidden="true"
                />
              ) : (
                <IconChevronDown
                  data-testid="workflow-agent-model-chevron"
                  className="size-3.5 shrink-0 opacity-50"
                />
              )}
            </span>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-56 p-0">
            <Command>
              <CommandInput
                aria-label={t("settings.workflow.searchAvailableAgentModels")}
                placeholder={t("settings.workflow.searchAvailableAgentModels")}
                className="text-sm"
              />
              <CommandList className="max-h-72">
                <CommandEmpty className="py-6 text-center text-xs">
                  <div className="space-y-2">
                    <p>
                      {modelsLoading
                        ? t("chat.modelSelector.loading")
                        : t("settings.workflow.noAvailableAgentModels")}
                    </p>
                    {modelsError && onRetryModels !== undefined && (
                      <Button
                        type="button"
                        variant="secondary"
                        size="sm"
                        onClick={onRetryModels}
                      >
                        {t("common.retry")}
                      </Button>
                    )}
                  </div>
                </CommandEmpty>
                <CommandGroup
                  heading={t("chat.modelSelector.agent")}
                  className="**:[[cmdk-group-heading]]:font-normal"
                >
                  {offeredAgents.map((agent) => {
                    const cliLoading =
                      cliStatus?.[agent.agentRef]?.isLoading === true;
                    return (
                      <CommandItem
                        key={agent.agentRef}
                        value={`${agent.label} agent`}
                        className="gap-1.5 rounded-sm px-2 py-1.5 text-xs"
                        onSelect={() => selectAgentCli(agent.agentRef)}
                      >
                        <PluginLogoMark
                          logo={agent.logo}
                          fallback={IconRobot}
                          className="size-3.5 object-contain"
                        />
                        {agent.label}
                        {cliLoading ? (
                          <IconLoader2 className="ml-auto size-3.5 shrink-0 animate-spin opacity-50" />
                        ) : agent.agentRef === currentAgentCli ? (
                          <IconCheck className="ml-auto size-4" />
                        ) : null}
                      </CommandItem>
                    );
                  })}
                </CommandGroup>
                <CommandGroup
                  heading={t("chat.modelSelector.model")}
                  className="**:[[cmdk-group-heading]]:font-normal"
                >
                  {modelsForSelectedCli.length === 0 ? (
                    <p className="px-2 py-4 text-center text-xs text-muted-foreground">
                      {t(
                        selectedCliLoading
                          ? "chat.modelSelector.loading"
                          : "settings.workflow.noAvailableAgentModels",
                      )}
                    </p>
                  ) : (
                    modelsForSelectedCli.map((model) => {
                      const name = workflowModelDisplayName(
                        model,
                        offeredAgents,
                      );
                      return (
                        <CommandItem
                          key={`${model.agentCli}:${model.modelId}`}
                          value={`${name} ${model.modelId}`}
                          className="gap-1.5 rounded-sm px-2 py-1.5 text-xs whitespace-normal"
                          onSelect={() => {
                            onChange({
                              executor: {
                                agentCli: model.agentCli,
                                modelId: model.modelId,
                              },
                            });
                            setModelPickerOpen(false);
                          }}
                        >
                          {name}
                          {model.modelId === config.executor.modelId && (
                            <IconCheck className="ml-auto size-4 shrink-0" />
                          )}
                        </CommandItem>
                      );
                    })
                  )}
                </CommandGroup>
              </CommandList>
            </Command>
          </PopoverContent>
        </Popover>
      </InspectorField>
      <InspectorField
        label={t("settings.workflow.field.prompt")}
        htmlFor="workflow-agent-prompt"
      >
        <WorkflowPromptEditor
          value={config.prompt}
          variableCatalog={variableCatalog}
          ariaLabel={t("settings.workflow.field.prompt")}
          insertVariableLabel={t("settings.workflow.field.insertVariable")}
          onChange={(prompt) => onChange({ prompt })}
        />
      </InspectorField>
    </>
  );
}
