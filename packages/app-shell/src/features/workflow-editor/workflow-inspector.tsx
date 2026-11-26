import { WorkflowMcpFields } from "./workflow-mcp-fields";
import { WorkflowAgentRetryFields } from "./workflow-agent-retry-fields";
import { WorkflowAgentCompletionFields } from "./workflow-agent-completion-fields";
import { WorkflowAgentInactivityField } from "./workflow-agent-inactivity-field";
import { WorkflowAgentCompletionField } from "./workflow-agent-completion-field";
import type { WorkflowMcpCatalogStatus } from "./mcp-catalog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconChevronDown,
  IconPlus,
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
  Label,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Select,
  SelectContent,
  SelectTrigger,
  SelectValue,
  Switch,
} from "@ora/ui";
import {
  DEFAULT_WORKFLOW_STRUCTURED_OUTPUT_SCHEMA,
  type WorkflowAgentConfig,
  type WorkflowAgentModel,
  type WorkflowNodeData,
  type WorkflowCapabilities,
  type WorkflowOutputBinding,
  type WorkflowVariableCatalogEntry,
  type WorkflowGlobalVariable,
  normalizeWorkflowAgentConfig,
  workflowAgentRetryApplies,
} from "@ora/workflow-mock";
import type { Node } from "@xyflow/react";
import { type AgentEntry } from "../../state/hooks/use-agent-catalog";
import { AgentExecutionFields } from "./workflow-agent-execution-fields";
import type { WorkflowAgentCliStatus } from "../../state/hooks/use-workflow-agent-models";
import {
  InspectorField,
  WorkflowNodeDetailsHeader,
  WorkflowNodeDetailsLayout,
} from "./workflow-node-details";
import { WorkflowVariableDisplay } from "./workflow-variable-display";
import { WorkflowVariableSelectGroups } from "./workflow-variable-list";
import {
  WorkflowStructuredOutputDialog,
  WorkflowStructuredOutputSummary,
} from "./workflow-structured-output-dialog";

interface WorkflowInspectorProps {
  node: Node<WorkflowNodeData, "workflow"> | null;
  capabilities: WorkflowCapabilities;
  mcpCatalog?: WorkflowMcpCatalogStatus;
  variableCatalog: WorkflowVariableCatalogEntry[];
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
  /** Whole-graph nodes, for detail panels whose configuration reads graph structure. */
  graphNodes?: Node<WorkflowNodeData, "workflow">[];
  /** Workflow-wide declarations used by graph-aware detail panels. */
  globalVariables?: WorkflowGlobalVariable[];
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
      mcpCatalog={props.mcpCatalog}
      variableCatalog={props.variableCatalog}
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
      graphNodes={props.graphNodes}
      globalVariables={props.globalVariables}
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
  mcpCatalog,
  variableCatalog,
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
  graphNodes,
  globalVariables,
}: {
  node: Node<WorkflowNodeData, "workflow">;
  capabilities: WorkflowCapabilities;
  mcpCatalog?: WorkflowMcpCatalogStatus;
  variableCatalog: WorkflowVariableCatalogEntry[];
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
  /** Whole-graph nodes, for detail panels whose configuration reads graph structure. */
  graphNodes?: Node<WorkflowNodeData, "workflow">[];
  globalVariables?: WorkflowGlobalVariable[];
}) {
  const { t } = useTranslation();
  const nodeType = capabilities.nodeTypes.find(
    (candidate) => candidate.kind === node.data.kind,
  );
  if (nodeType === undefined) {
    throw new Error(
      `Missing workflow capability for node kind "${node.data.kind}"`,
    );
  }
  const agentConfig = node.data.agentConfig;
  // Agent and output keep their dedicated flat editors; the remaining kinds
  // use the Dify-style grouped layout so their details read as sections.
  const usesFlatLayout =
    node.data.kind === "agent" || node.data.kind === "output";
  const outputBindings =
    node.data.kind === "output" ? (node.data.outputs ?? []) : [];
  const updateOutputBindings = (next: WorkflowOutputBinding[]): void => {
    onUpdate({ ...node, data: { ...node.data, outputs: next } });
  };
  const updateOutputBinding = (
    index: number,
    patch: Partial<WorkflowOutputBinding>,
  ): void => {
    updateOutputBindings(
      outputBindings.map((binding, candidateIndex) =>
        candidateIndex === index ? { ...binding, ...patch } : binding,
      ),
    );
  };
  return (
    <aside
      data-workflow-inspector=""
      className="flex h-full min-h-0 w-full min-w-0 flex-1 flex-col overflow-hidden border-l border-border bg-background"
    >
      {usesFlatLayout ? (
        <>
          <WorkflowNodeDetailsHeader
            node={node}
            nodeType={nodeType}
            onUpdate={onUpdate}
            onClose={onClose}
          />
          <div className="min-h-0 min-w-0 flex-1 space-y-4 overflow-x-hidden overflow-y-auto p-4">
            {nodeType.configFields.includes("agent") && agentConfig != null && (
              <AgentConfigurationFields
                key={node.id}
                config={agentConfig}
                capabilities={capabilities}
                mcpCatalog={mcpCatalog}
                modelsLoading={agentModelsLoading}
                modelsError={agentModelsError}
                onRetryModels={onRetryAgentModels}
                agents={agents}
                modelsByCli={modelsByCli}
                cliStatus={cliStatus}
                variableCatalog={variableCatalog}
                catalogsLoading={agentCatalogsLoading}
                catalogsError={agentCatalogsError}
                onRetryCatalogs={onRetryAgentCatalogs}
                iterationMember={node.parentId !== undefined}
                onChange={(config) =>
                  onUpdate({
                    ...node,
                    data: { ...node.data, agentConfig: config },
                  })
                }
              />
            )}
            {node.data.kind === "output" && (
              <InspectorField
                label={t("settings.workflow.field.outputBindings")}
                htmlFor="workflow-output-bindings"
              >
                <div className="space-y-2">
                  {outputBindings.map((binding, bindingIndex) => (
                    <div
                      className="flex items-start gap-1.5"
                      key={bindingIndex}
                    >
                      <div className="min-w-0 flex-1 space-y-1.5 rounded-lg bg-muted/70 p-2">
                        <Input
                          value={binding.name}
                          aria-label={t(
                            "settings.workflow.field.outputBindingName",
                            { index: bindingIndex + 1 },
                          )}
                          placeholder={t(
                            "settings.workflow.field.outputBindingNamePlaceholder",
                          )}
                          className="h-8 w-full bg-background"
                          onChange={(event) =>
                            updateOutputBinding(bindingIndex, {
                              name: event.target.value,
                            })
                          }
                        />
                        <Select
                          value={binding.variableSelector.join(".")}
                          onValueChange={(value) => {
                            if (value !== null) {
                              updateOutputBinding(bindingIndex, {
                                variableSelector: value
                                  .split(".")
                                  .map((part) => part.trim())
                                  .filter((part) => part !== ""),
                              });
                            }
                          }}
                        >
                          <SelectTrigger
                            className="h-8 w-full bg-background"
                            aria-label={t(
                              "settings.workflow.field.outputBindingSelector",
                              { index: bindingIndex + 1 },
                            )}
                          >
                            <SelectValue placeholder="node.variable.path">
                              {(() => {
                                const selector =
                                  binding.variableSelector.join(".");
                                const variable = variableCatalog.find(
                                  (candidate) =>
                                    candidate.selector.join(".") === selector,
                                );
                                if (variable === undefined) {
                                  return selector;
                                }
                                return (
                                  <WorkflowVariableDisplay
                                    variable={variable}
                                    nodeName={
                                      variable.sourceNodeTitle ??
                                      (variable.scope === "global"
                                        ? t("settings.workflow.globalVariables")
                                        : variable.sourceNodeId)
                                    }
                                  />
                                );
                              })()}
                            </SelectValue>
                          </SelectTrigger>
                          <SelectContent
                            alignItemWithTrigger={false}
                            align="start"
                            className="w-70 min-w-70 max-w-70"
                          >
                            <WorkflowVariableSelectGroups
                              variables={variableCatalog}
                              globalVariablesLabel={t(
                                "settings.workflow.globalVariables",
                              )}
                            />
                          </SelectContent>
                        </Select>
                      </div>
                      {outputBindings.length > 1 && (
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-sm"
                          className="mt-1 shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                          aria-label={t(
                            "settings.workflow.field.outputBindingRemove",
                            { index: bindingIndex + 1 },
                          )}
                          onClick={() =>
                            updateOutputBindings(
                              outputBindings.filter(
                                (_, candidateIndex) =>
                                  candidateIndex !== bindingIndex,
                              ),
                            )
                          }
                        >
                          <IconTrash className="size-3.5" />
                        </Button>
                      )}
                    </div>
                  ))}
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="w-full justify-start"
                    onClick={() =>
                      updateOutputBindings([
                        ...outputBindings,
                        {
                          name: `result-${outputBindings.length + 1}`,
                          variableSelector: [],
                        },
                      ])
                    }
                  >
                    <IconPlus />
                    {t("settings.workflow.field.outputBindingAdd")}
                  </Button>
                </div>
              </InspectorField>
            )}
          </div>
        </>
      ) : (
        <WorkflowNodeDetailsLayout
          node={node}
          nodeType={nodeType}
          capabilities={capabilities}
          variableCatalog={variableCatalog}
          onUpdate={onUpdate}
          onClose={onClose}
          graphNodes={graphNodes}
          globalVariables={globalVariables}
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

/** Edits optional structured parsing without changing the node's persisted raw output. */
function AgentConfigurationFields({
  config: rawConfig,
  capabilities,
  mcpCatalog,
  modelsLoading,
  modelsError,
  onRetryModels,
  agents,
  modelsByCli,
  cliStatus,
  variableCatalog,
  catalogsLoading,
  catalogsError,
  onRetryCatalogs,
  iterationMember,
  onChange,
}: {
  config: WorkflowAgentConfig;
  capabilities: WorkflowCapabilities;
  mcpCatalog?: WorkflowMcpCatalogStatus;
  modelsLoading: boolean;
  modelsError: boolean;
  onRetryModels?: () => void;
  agents?: AgentEntry[];
  modelsByCli?: ReadonlyMap<string, WorkflowAgentModel[]>;
  cliStatus?: Readonly<Record<string, WorkflowAgentCliStatus>>;
  variableCatalog: WorkflowVariableCatalogEntry[];
  catalogsLoading: boolean;
  catalogsError: boolean;
  onRetryCatalogs?: () => void;
  iterationMember: boolean;
  onChange: (config: WorkflowAgentConfig) => void;
}) {
  const { t } = useTranslation();
  const [rolePickerOpen, setRolePickerOpen] = useState(false);
  const [skillPickerOpen, setSkillPickerOpen] = useState(false);
  const [structuredOutputDialogOpen, setStructuredOutputDialogOpen] =
    useState(false);
  // Older drafts may omit `mcps`; normalize before any list access.
  const config = normalizeWorkflowAgentConfig(rawConfig);
  // Alias the narrowed structured setting so spreading it keeps its discriminant.
  const structuredContract =
    config.outputContract?.type === "structured" ? config.outputContract : null;
  const configuredSkillIds = new Set(
    config.skills.map((skill) => skill.skillId),
  );
  const availableSkills = capabilities.skills.filter(
    (skill) => !configuredSkillIds.has(skill.value),
  );
  const enabledSkillCount = config.skills.filter(
    (skill) => skill.enabled,
  ).length;
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

  return (
    <>
      <AgentExecutionFields
        {...{
          config,
          capabilities,
          modelsLoading,
          modelsError,
          onRetryModels,
          agents,
          modelsByCli,
          cliStatus,
          variableCatalog,
        }}
        onChange={(patch) => onChange({ ...config, ...patch })}
      />
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
        <p className="text-[10px] leading-relaxed text-muted-foreground">
          {t("settings.workflow.skillRequirementHint")}
        </p>
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
      <WorkflowMcpFields
        config={config}
        choices={capabilities.mcps ?? []}
        catalog={mcpCatalog}
        onChange={onChange}
      />
      <InspectorField
        label={t("settings.workflow.field.interactive")}
        htmlFor="workflow-agent-interactive"
      >
        <div className="space-y-2">
          <div className="flex items-center justify-between gap-3">
            <p className="text-xs text-muted-foreground">
              {t(
                iterationMember
                  ? "settings.workflow.iteration.interactiveUnavailable"
                  : "settings.workflow.field.interactiveDescription",
              )}
            </p>
            <Switch
              id="workflow-agent-interactive"
              className="shrink-0 data-checked:bg-blue-600 hover:data-checked:bg-blue-700"
              checked={config.interactive ?? false}
              disabled={iterationMember}
              onCheckedChange={(interactive) =>
                onChange(
                  interactive
                    ? {
                        ...config,
                        interactive,
                        // New interactive nodes start on the explicit signal; an explicit
                        // `end_turn` choice already on the config stays as the user saved it.
                        ...(config.completionSignal == null
                          ? {
                              schemaVersion: 4,
                              completionSignal: "agent" as const,
                            }
                          : {}),
                      }
                    : {
                        ...config,
                        interactive,
                        // The signal and auto-accept only exist for an interactive node; leaving
                        // them behind would give a non-interactive prompt an unusable token.
                        completionSignal: undefined,
                        autoAcceptAgentCompletion: undefined,
                      },
                )
              }
            />
          </div>
          {iterationMember && config.interactive === true && (
            <div
              role="alert"
              className="rounded-md border border-destructive/30 bg-destructive/5 p-2 text-xs text-destructive"
            >
              <p>{t("settings.workflow.iteration.interactiveRepairHint")}</p>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="mt-2 h-7"
                onClick={() =>
                  onChange({
                    ...config,
                    interactive: false,
                    completionSignal: undefined,
                    autoAcceptAgentCompletion: undefined,
                  })
                }
              >
                {t("settings.workflow.iteration.disableInteractive")}
              </Button>
            </div>
          )}
        </div>
      </InspectorField>
      <WorkflowAgentInactivityField config={config} onChange={onChange} />
      {config.interactive === true && (
        <WorkflowAgentCompletionField config={config} onChange={onChange} />
      )}
      <div className="min-w-0 space-y-1.5">
        <div className="flex items-center justify-between gap-3">
          <Label
            htmlFor="workflow-agent-output-contract"
            className="text-[11px]"
          >
            {t("settings.workflow.field.structuredOutput")}
          </Label>
          <Switch
            id="workflow-agent-output-contract"
            checked={structuredContract !== null}
            onCheckedChange={(enabled) => {
              if (!enabled) {
                onChange({
                  ...config,
                  outputContract: undefined,
                });
                return;
              }
              const schema = structuredClone(
                DEFAULT_WORKFLOW_STRUCTURED_OUTPUT_SCHEMA,
              );
              onChange({
                ...config,
                outputContract: { type: "structured", schema },
              });
            }}
          />
        </div>
        {structuredContract !== null && (
          <div className="pt-1">
            <WorkflowStructuredOutputSummary
              schema={structuredContract.schema}
              onConfigure={() => setStructuredOutputDialogOpen(true)}
            />
            <WorkflowStructuredOutputDialog
              open={structuredOutputDialogOpen}
              schema={structuredContract.schema}
              onOpenChange={setStructuredOutputDialogOpen}
              onSave={(schema) =>
                onChange({
                  ...config,
                  outputContract: { type: "structured", schema },
                })
              }
            />
          </div>
        )}
      </div>
      {(config.interactive === true || config.completion !== undefined) && (
        <WorkflowAgentCompletionFields config={config} onChange={onChange} />
      )}
      {/* Interactive nodes wait for a person instead of rerunning, so the setting would be inert. */}
      {workflowAgentRetryApplies(config) && (
        <WorkflowAgentRetryFields config={config} onChange={onChange} />
      )}
    </>
  );
}
