import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconCheck,
  IconLayoutSidebarRightCollapse,
  IconChevronDown,
  IconLoader2,
  IconPlus,
  IconRobot,
  IconSettings,
  IconTrash,
} from "@tabler/icons-react";
import {
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  Input,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
  Textarea,
} from "@ora/ui";
import {
  type WorkflowAgentConfig,
  type WorkflowAgentModel,
  type WorkflowNodeData,
  type WorkflowCapabilities,
  normalizeWorkflowAgentConfig,
} from "@ora/workflow-mock";
import type { Node } from "@xyflow/react";
import { agentLabel, type AgentEntry } from "../chat/agent-catalog";
import { PluginLogoMark } from "../settings/plugin-logo";
import type { WorkflowAgentCliStatus } from "../../state/hooks/use-workflow-agent-models";
import { getNodeMetadata } from "./workflow-node-metadata";
import {
  InspectorField,
  WorkflowNodeDetailsLayout,
} from "./workflow-node-details";

/** Soft card copy limit so node descriptions stay glanceable on the canvas. */
const NODE_DESCRIPTION_MAX_LENGTH = 30;

interface WorkflowInspectorProps {
  node: Node<WorkflowNodeData, "workflow"> | null;
  capabilities: WorkflowCapabilities;
  agentModelsLoading?: boolean;
  agentModelsError?: boolean;
  onRetryAgentModels?: () => void;
  agents?: AgentEntry[];
  modelsByCli?: ReadonlyMap<string, WorkflowAgentModel[]>;
  cliStatus?: Readonly<Record<string, WorkflowAgentCliStatus>>;
  agentCatalogsLoading?: boolean;
  agentCatalogsError?: boolean;
  onRetryAgentCatalogs?: () => void;
  onUpdate: (node: Node<WorkflowNodeData, "workflow">) => void;
  onDelete: (nodeId: string) => void;
  onCloseNode: () => void;
}

/** Right-rail editor for the selected workflow node (definition only). */
export function WorkflowInspector(props: WorkflowInspectorProps) {
  if (props.node === null) {
    return <WorkflowInspectorEmpty />;
  }
  return (
    <WorkflowNodeInspector
      node={props.node}
      capabilities={props.capabilities}
      agentModelsLoading={props.agentModelsLoading ?? false}
      agentModelsError={props.agentModelsError ?? false}
      onRetryAgentModels={props.onRetryAgentModels}
      agents={props.agents}
      modelsByCli={props.modelsByCli}
      cliStatus={props.cliStatus}
      agentCatalogsLoading={props.agentCatalogsLoading ?? false}
      agentCatalogsError={props.agentCatalogsError ?? false}
      onRetryAgentCatalogs={props.onRetryAgentCatalogs}
      onUpdate={props.onUpdate}
      onDelete={props.onDelete}
      onClose={props.onCloseNode}
    />
  );
}

/** Shown when the inspector is open but no node is selected. */
function WorkflowInspectorEmpty() {
  const { t } = useTranslation();
  return (
    <aside className="flex h-full min-h-0 w-full min-w-0 flex-1 flex-col overflow-hidden border-l border-border bg-background">
      <div className="border-b border-border px-4 py-3">
        <h3 className="text-xs font-semibold">
          {t("settings.workflow.configuration")}
        </h3>
        <p className="mt-1 text-[11px] text-muted-foreground">
          {t("settings.workflow.selectNodeHint")}
        </p>
      </div>
      <div className="flex flex-1 flex-col items-center justify-center px-6 text-center">
        <span className="mb-3 flex size-10 items-center justify-center rounded-xl bg-muted">
          <IconSettings className="size-5 text-muted-foreground" />
        </span>
        <p className="text-xs font-medium">
          {t("settings.workflow.noSelection")}
        </p>
        <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
          {t("settings.workflow.noSelectionHint")}
        </p>
      </div>
    </aside>
  );
}

/** Edits a node in place with visible labels and progressive, kind-specific fields. */
function WorkflowNodeInspector({
  node,
  capabilities,
  agentModelsLoading,
  agentModelsError,
  onRetryAgentModels,
  agents,
  modelsByCli,
  cliStatus,
  agentCatalogsLoading,
  agentCatalogsError,
  onRetryAgentCatalogs,
  onUpdate,
  onDelete,
  onClose,
}: {
  node: Node<WorkflowNodeData, "workflow">;
  capabilities: WorkflowCapabilities;
  agentModelsLoading: boolean;
  agentModelsError: boolean;
  onRetryAgentModels?: () => void;
  agents?: AgentEntry[];
  modelsByCli?: ReadonlyMap<string, WorkflowAgentModel[]>;
  cliStatus?: Readonly<Record<string, WorkflowAgentCliStatus>>;
  agentCatalogsLoading: boolean;
  agentCatalogsError: boolean;
  onRetryAgentCatalogs?: () => void;
  onUpdate: (node: Node<WorkflowNodeData, "workflow">) => void;
  onDelete: (nodeId: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const metadata = getNodeMetadata(node.data.kind);
  const nodeType = capabilities.nodeTypes.find(
    (candidate) => candidate.kind === node.data.kind,
  );
  if (nodeType === undefined) {
    throw new Error(
      `Missing workflow capability for node kind "${node.data.kind}"`,
    );
  }
  const Icon = metadata.icon;
  const agentConfig = node.data.agentConfig;
  // Agent and output keep their dedicated flat editors; the remaining kinds
  // use the Dify-style grouped layout so their details read as sections.
  const usesFlatLayout =
    node.data.kind === "agent" || node.data.kind === "output";
  return (
    <aside
      data-workflow-inspector=""
      className="flex h-full min-h-0 w-full min-w-0 flex-1 flex-col overflow-hidden border-l border-border bg-background"
    >
      {usesFlatLayout ? (
        <>
          <div className="flex min-w-0 items-center gap-2.5 border-b border-border px-4 py-3">
            <span
              className={`flex size-8 items-center justify-center rounded-lg ${metadata.tone}`}
            >
              <Icon className="size-4" />
            </span>
            <div className="min-w-0 flex-1">
              <h3 className="truncate text-xs font-semibold">
                {node.data.title}
              </h3>
              <p className="text-[10px] text-muted-foreground">
                {t("settings.workflow.nodeSuffix", { type: nodeType.label })}
              </p>
            </div>
            <Button
              variant="ghost"
              size="icon-sm"
              className="shrink-0"
              aria-label={t("settings.workflow.closeConfiguration")}
              onClick={onClose}
            >
              <IconLayoutSidebarRightCollapse />
            </Button>
          </div>
          <div className="min-h-0 min-w-0 flex-1 space-y-4 overflow-x-hidden overflow-y-auto p-4">
            <InspectorField
              label={t("settings.workflow.field.name")}
              htmlFor="workflow-node-title"
            >
              <Input
                id="workflow-node-title"
                value={node.data.title}
                onChange={(event) =>
                  onUpdate({
                    ...node,
                    data: { ...node.data, title: event.target.value },
                  })
                }
              />
            </InspectorField>
            <InspectorField
              label={t("settings.workflow.field.description")}
              htmlFor="workflow-node-description"
            >
              <>
                <Input
                  id="workflow-node-description"
                  value={node.data.description}
                  maxLength={NODE_DESCRIPTION_MAX_LENGTH}
                  onChange={(event) =>
                    onUpdate({
                      ...node,
                      data: {
                        ...node.data,
                        description: event.target.value.slice(
                          0,
                          NODE_DESCRIPTION_MAX_LENGTH,
                        ),
                      },
                    })
                  }
                />
                <p
                  className="text-right text-[10px] text-muted-foreground"
                  aria-live="polite"
                >
                  {t("settings.workflow.characterCount", {
                    count: node.data.description.length,
                    max: NODE_DESCRIPTION_MAX_LENGTH,
                  })}
                </p>
              </>
            </InspectorField>
            {nodeType.configFields.includes("agent") &&
              agentConfig !== undefined && (
                <AgentConfigurationFields
                  config={agentConfig}
                  capabilities={capabilities}
                  modelsLoading={agentModelsLoading}
                  modelsError={agentModelsError}
                  onRetryModels={onRetryAgentModels}
                  agents={agents}
                  modelsByCli={modelsByCli}
                  cliStatus={cliStatus}
                  catalogsLoading={agentCatalogsLoading}
                  catalogsError={agentCatalogsError}
                  onRetryCatalogs={onRetryAgentCatalogs}
                  onChange={(config) =>
                    onUpdate({
                      ...node,
                      data: { ...node.data, agentConfig: config },
                    })
                  }
                />
              )}
            {nodeType.configFields.includes("instruction") && (
              <InspectorField
                label={t("settings.workflow.field.instruction")}
                htmlFor="workflow-node-instruction"
              >
                <Textarea
                  id="workflow-node-instruction"
                  className="min-h-32 resize-none text-xs leading-5"
                  value={node.data.instruction ?? ""}
                  onChange={(event) =>
                    onUpdate({
                      ...node,
                      data: { ...node.data, instruction: event.target.value },
                    })
                  }
                />
              </InspectorField>
            )}
          </div>
        </>
      ) : (
        <WorkflowNodeDetailsLayout
          node={node}
          nodeType={nodeType}
          capabilities={capabilities}
          onUpdate={onUpdate}
          onClose={onClose}
        />
      )}
      <div className="border-t border-border p-3">
        <Button
          variant="ghost"
          className="w-full justify-start text-destructive hover:bg-destructive/10 hover:text-destructive"
          onClick={() => onDelete(node.id)}
          disabled={node.data.kind === "start"}
        >
          <IconTrash />
          {t("settings.workflow.deleteNode")}
        </Button>
      </div>
    </aside>
  );
}

/** Edits the structured Agent contract without conflating it with a free-form prompt field. */
function AgentConfigurationFields({
  config: rawConfig,
  capabilities,
  modelsLoading,
  modelsError,
  onRetryModels,
  agents,
  modelsByCli,
  cliStatus,
  catalogsLoading,
  catalogsError,
  onRetryCatalogs,
  onChange,
}: {
  config: WorkflowAgentConfig;
  capabilities: WorkflowCapabilities;
  modelsLoading: boolean;
  modelsError: boolean;
  onRetryModels?: () => void;
  agents?: AgentEntry[];
  modelsByCli?: ReadonlyMap<string, WorkflowAgentModel[]>;
  cliStatus?: Readonly<Record<string, WorkflowAgentCliStatus>>;
  catalogsLoading: boolean;
  catalogsError: boolean;
  onRetryCatalogs?: () => void;
  onChange: (config: WorkflowAgentConfig) => void;
}) {
  const { t } = useTranslation();
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [rolePickerOpen, setRolePickerOpen] = useState(false);
  const [skillPickerOpen, setSkillPickerOpen] = useState(false);
  const [mcpPickerOpen, setMcpPickerOpen] = useState(false);
  // Older drafts may omit `mcps`; normalize before any list access.
  const config = normalizeWorkflowAgentConfig(rawConfig);
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
  const configuredSkillIds = new Set(
    config.skills.map((skill) => skill.skillId),
  );
  const availableSkills = capabilities.skills.filter(
    (skill) => !configuredSkillIds.has(skill.value),
  );
  const enabledSkillCount = config.skills.filter(
    (skill) => skill.enabled,
  ).length;
  const configuredMcpIds = new Set(config.mcps.map((mcp) => mcp.mcpId));
  const availableMcps = (capabilities.mcps ?? []).filter(
    (mcp) => !configuredMcpIds.has(mcp.value),
  );
  const enabledMcpCount = config.mcps.filter((mcp) => mcp.enabled).length;
  const configuredRole = capabilities.roles.find(
    (role) => role.value === config.roleId,
  );
  const noRoleOption = { value: "", label: t("settings.workflow.noRole") };
  const selectedRole =
    configuredRole ??
    (config.roleId === ""
      ? noRoleOption
      : { value: config.roleId, label: config.roleId });
  // The empty option is always selectable; an out-of-catalog role stays visible so it can be re-picked.
  const selectableRoles =
    configuredRole === undefined && config.roleId !== ""
      ? [noRoleOption, selectedRole, ...capabilities.roles]
      : [noRoleOption, ...capabilities.roles];

  /** Adds a new Skill in its enabled state, preserving configuration order. */
  function addSkill(skillId: string): void {
    onChange({
      ...config,
      skills: [...config.skills, { skillId, enabled: true }],
    });
    setSkillPickerOpen(false);
  }

  /** Updates only the enabled state of a configured Skill. */
  function setSkillEnabled(skillId: string, enabled: boolean): void {
    onChange({
      ...config,
      skills: config.skills.map((skill) =>
        skill.skillId === skillId ? { ...skill, enabled } : skill,
      ),
    });
  }

  /** Removes a configured Skill without affecting the remaining selection order. */
  function removeSkill(skillId: string): void {
    onChange({
      ...config,
      skills: config.skills.filter((skill) => skill.skillId !== skillId),
    });
  }

  /** Adds a new MCP in its enabled state, preserving configuration order. */
  function addMcp(mcpId: string): void {
    onChange({
      ...config,
      mcps: [...config.mcps, { mcpId, enabled: true }],
    });
    setMcpPickerOpen(false);
  }

  /** Updates only the enabled state of a configured MCP. */
  function setMcpEnabled(mcpId: string, enabled: boolean): void {
    onChange({
      ...config,
      mcps: config.mcps.map((mcp) =>
        mcp.mcpId === mcpId ? { ...mcp, enabled } : mcp,
      ),
    });
  }

  /** Removes a configured MCP without affecting the remaining selection order. */
  function removeMcp(mcpId: string): void {
    onChange({
      ...config,
      mcps: config.mcps.filter((mcp) => mcp.mcpId !== mcpId),
    });
  }

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
      ...config,
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
                              ...config,
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
        label={t("settings.workflow.field.role")}
        htmlFor="workflow-agent-role"
      >
        <Popover open={rolePickerOpen} onOpenChange={setRolePickerOpen}>
          <PopoverTrigger
            render={
              <Button
                id="workflow-agent-role"
                type="button"
                variant="outline"
                className="h-9 w-full min-w-0 shrink justify-between overflow-hidden px-3 font-normal"
                disabled={catalogsLoading && selectableRoles.length === 0}
                aria-label={t("settings.workflow.field.role")}
              />
            }
          >
            <span className="flex w-full min-w-0 items-center justify-between gap-2">
              <span className="min-w-0 flex-1 truncate text-left">
                {selectedRole.label}
              </span>
              <IconChevronDown
                data-testid="workflow-agent-role-chevron"
                className="size-3.5 shrink-0 opacity-50"
              />
            </span>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-80 p-0">
            <Command>
              <CommandInput
                aria-label={t("settings.workflow.searchAvailableRoles")}
                placeholder={t("settings.workflow.searchAvailableRoles")}
                className="text-sm"
              />
              <CommandList className="max-h-60">
                <CommandEmpty className="py-6 text-center text-xs">
                  <div className="space-y-2">
                    <p>
                      {catalogsLoading
                        ? t("settings.roles.loading")
                        : t("settings.workflow.noAvailableRoles")}
                    </p>
                    {catalogsError && onRetryCatalogs !== undefined && (
                      <Button
                        type="button"
                        variant="secondary"
                        size="sm"
                        onClick={onRetryCatalogs}
                      >
                        {t("common.retry")}
                      </Button>
                    )}
                  </div>
                </CommandEmpty>
                <CommandGroup>
                  {selectableRoles.map((role) => (
                    <CommandItem
                      key={role.value}
                      value={`${role.label} ${role.value}`}
                      onSelect={() => {
                        onChange({ ...config, roleId: role.value });
                        setRolePickerOpen(false);
                      }}
                    >
                      {role.label}
                    </CommandItem>
                  ))}
                </CommandGroup>
              </CommandList>
            </Command>
          </PopoverContent>
        </Popover>
      </InspectorField>
      <fieldset className="min-w-0 space-y-2">
        <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
          <legend className="min-w-0 text-[11px] font-medium">
            {t("settings.workflow.field.skills")}
          </legend>
          <div className="flex shrink-0 items-center gap-1">
            <span className="whitespace-nowrap text-[10px] text-muted-foreground">
              {t("settings.workflow.enabledSkillCount", {
                enabled: enabledSkillCount,
                total: config.skills.length,
              })}
            </span>
            <Popover open={skillPickerOpen} onOpenChange={setSkillPickerOpen}>
              <PopoverTrigger
                render={
                  <Button
                    id="workflow-add-skill"
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    disabled={
                      catalogsLoading && capabilities.skills.length === 0
                    }
                    aria-label={t("settings.workflow.addSkill")}
                  />
                }
              >
                <IconPlus />
              </PopoverTrigger>
              <PopoverContent align="end" className="w-72 p-0">
                <Command>
                  <CommandInput
                    aria-label={t("settings.workflow.searchAvailableSkills")}
                    placeholder={t("settings.workflow.searchAvailableSkills")}
                    className="text-sm"
                  />
                  <CommandList className="max-h-60">
                    <CommandEmpty className="py-6 text-center text-xs">
                      <div className="space-y-2">
                        <p>
                          {catalogsLoading
                            ? t("settings.skills.loading")
                            : t("settings.workflow.noAvailableSkills")}
                        </p>
                        {catalogsError && onRetryCatalogs !== undefined && (
                          <Button
                            type="button"
                            variant="secondary"
                            size="sm"
                            onClick={onRetryCatalogs}
                          >
                            {t("common.retry")}
                          </Button>
                        )}
                      </div>
                    </CommandEmpty>
                    <CommandGroup>
                      {availableSkills.map((skill) => (
                        <CommandItem
                          key={skill.value}
                          value={`${skill.label} ${skill.value}`}
                          onSelect={() => addSkill(skill.value)}
                        >
                          {skill.label}
                        </CommandItem>
                      ))}
                    </CommandGroup>
                  </CommandList>
                </Command>
              </PopoverContent>
            </Popover>
          </div>
        </div>
        <div className="min-w-0 divide-y overflow-hidden rounded-md border border-border">
          {config.skills.map((configuredSkill) => {
            const skill = capabilities.skills.find(
              (candidate) => candidate.value === configuredSkill.skillId,
            ) ?? {
              value: configuredSkill.skillId,
              label: configuredSkill.skillId,
            };
            return (
              <div
                key={configuredSkill.skillId}
                className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-2 px-2.5 py-2"
              >
                <span className="min-w-0 truncate text-xs">{skill.label}</span>
                <Switch
                  size="sm"
                  className="shrink-0 data-checked:bg-blue-600 hover:data-checked:bg-blue-700"
                  checked={configuredSkill.enabled}
                  aria-label={t("settings.workflow.toggleSkill", {
                    name: skill.label,
                  })}
                  onCheckedChange={(enabled) =>
                    setSkillEnabled(configuredSkill.skillId, enabled)
                  }
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                  aria-label={t("settings.workflow.removeSkill", {
                    name: skill.label,
                  })}
                  onClick={() => removeSkill(configuredSkill.skillId)}
                >
                  <IconTrash />
                </Button>
              </div>
            );
          })}
          {config.skills.length === 0 && (
            <p className="px-2.5 py-3 text-xs text-muted-foreground">
              {t("settings.workflow.noConfiguredSkills")}
            </p>
          )}
        </div>
      </fieldset>
      <fieldset className="min-w-0 space-y-2">
        <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
          <legend className="min-w-0 text-[11px] font-medium">
            {t("settings.workflow.field.mcps")}
          </legend>
          <div className="flex shrink-0 items-center gap-1">
            <span className="whitespace-nowrap text-[10px] text-muted-foreground">
              {t("settings.workflow.enabledMcpCount", {
                enabled: enabledMcpCount,
                total: config.mcps.length,
              })}
            </span>
            <Popover open={mcpPickerOpen} onOpenChange={setMcpPickerOpen}>
              <PopoverTrigger
                render={
                  <Button
                    id="workflow-add-mcp"
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    disabled={(capabilities.mcps ?? []).length === 0}
                    aria-label={t("settings.workflow.addMcp")}
                  />
                }
              >
                <IconPlus />
              </PopoverTrigger>
              <PopoverContent align="end" className="w-72 p-0">
                <Command>
                  <CommandInput
                    aria-label={t("settings.workflow.searchAvailableMcps")}
                    placeholder={t("settings.workflow.searchAvailableMcps")}
                    className="text-sm"
                  />
                  <CommandList className="max-h-60">
                    <CommandEmpty className="py-6 text-center text-xs">
                      {t("settings.workflow.noAvailableMcps")}
                    </CommandEmpty>
                    <CommandGroup>
                      {availableMcps.map((mcp) => (
                        <CommandItem
                          key={mcp.value}
                          value={`${mcp.label} ${mcp.value}`}
                          onSelect={() => addMcp(mcp.value)}
                        >
                          {mcp.label}
                        </CommandItem>
                      ))}
                    </CommandGroup>
                  </CommandList>
                </Command>
              </PopoverContent>
            </Popover>
          </div>
        </div>
        <div className="min-w-0 divide-y overflow-hidden rounded-md border border-border">
          {config.mcps.map((configuredMcp) => {
            const mcp = (capabilities.mcps ?? []).find(
              (candidate) => candidate.value === configuredMcp.mcpId,
            ) ?? { value: configuredMcp.mcpId, label: configuredMcp.mcpId };
            return (
              <div
                key={configuredMcp.mcpId}
                className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-2 px-2.5 py-2"
              >
                <span className="min-w-0 truncate text-xs">{mcp.label}</span>
                <Switch
                  size="sm"
                  className="shrink-0 data-checked:bg-blue-600 hover:data-checked:bg-blue-700"
                  checked={configuredMcp.enabled}
                  aria-label={t("settings.workflow.toggleMcp", {
                    name: mcp.label,
                  })}
                  onCheckedChange={(enabled) =>
                    setMcpEnabled(configuredMcp.mcpId, enabled)
                  }
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                  aria-label={t("settings.workflow.removeMcp", {
                    name: mcp.label,
                  })}
                  onClick={() => removeMcp(configuredMcp.mcpId)}
                >
                  <IconTrash />
                </Button>
              </div>
            );
          })}
          {config.mcps.length === 0 && (
            <p className="px-2.5 py-3 text-xs text-muted-foreground">
              {t("settings.workflow.noConfiguredMcps")}
            </p>
          )}
        </div>
      </fieldset>
      <InspectorField
        label={t("settings.workflow.field.prompt")}
        htmlFor="workflow-agent-prompt"
      >
        <Textarea
          id="workflow-agent-prompt"
          className="min-h-32 resize-none text-xs leading-5"
          value={config.prompt}
          onChange={(event) =>
            onChange({ ...config, prompt: event.target.value })
          }
        />
      </InspectorField>
      <InspectorField
        label={t("settings.workflow.field.interactive")}
        htmlFor="workflow-agent-interactive"
      >
        <div className="flex items-center justify-between gap-3">
          <p className="text-xs text-muted-foreground">
            {t("settings.workflow.field.interactiveDescription")}
          </p>
          <Switch
            id="workflow-agent-interactive"
            className="shrink-0 data-checked:bg-blue-600 hover:data-checked:bg-blue-700"
            checked={config.interactive ?? false}
            onCheckedChange={(interactive) =>
              onChange({ ...config, interactive })
            }
          />
        </div>
      </InspectorField>
      <InspectorField
        label={t("settings.workflow.field.outputPolicy")}
        htmlFor="workflow-agent-output-policy"
      >
        <Select
          value={config.outputPolicy ?? "none"}
          onValueChange={(value) =>
            onChange({
              ...config,
              // Only two policies exist; fold any unexpected value back to the default.
              outputPolicy: value === "none" ? "none" : "final_agent_response",
            })
          }
        >
          <SelectTrigger id="workflow-agent-output-policy" className="w-full">
            <SelectValue>
              {(selected) =>
                selected === "none"
                  ? t("settings.workflow.field.outputPolicyNone")
                  : t("settings.workflow.field.outputPolicyFinalAgentResponse")
              }
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="final_agent_response">
              {t("settings.workflow.field.outputPolicyFinalAgentResponse")}
            </SelectItem>
            <SelectItem value="none">
              {t("settings.workflow.field.outputPolicyNone")}
            </SelectItem>
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground">
          {t("settings.workflow.field.outputPolicyDescription")}
        </p>
      </InspectorField>
    </>
  );
}

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
