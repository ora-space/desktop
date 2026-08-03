import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconLayoutSidebarRightCollapse,
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
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
  Textarea,
} from "@ora/ui";
import {
  type WorkflowAgentConfig,
  type WorkflowNodeData,
  type WorkflowCapabilities,
} from "@ora/workflow-mock";
import type { Node } from "@xyflow/react";
import { getNodeMetadata } from "./workflow-node-metadata";

interface WorkflowInspectorProps {
  node: Node<WorkflowNodeData, "workflow"> | null;
  capabilities: WorkflowCapabilities;
  agentModelsLoading?: boolean;
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
    <aside className="flex min-h-0 flex-1 flex-col border-l border-border bg-background">
      <div className="border-b border-border px-4 py-3">
        <h3 className="text-xs font-semibold">{t("settings.workflow.configuration")}</h3>
        <p className="mt-1 text-[11px] text-muted-foreground">{t("settings.workflow.selectNodeHint")}</p>
      </div>
      <div className="flex flex-1 flex-col items-center justify-center px-6 text-center">
        <span className="mb-3 flex size-10 items-center justify-center rounded-xl bg-muted">
          <IconSettings className="size-5 text-muted-foreground" />
        </span>
        <p className="text-xs font-medium">{t("settings.workflow.noSelection")}</p>
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
  onUpdate,
  onDelete,
  onClose,
}: {
  node: Node<WorkflowNodeData, "workflow">;
  capabilities: WorkflowCapabilities;
  agentModelsLoading: boolean;
  onUpdate: (node: Node<WorkflowNodeData, "workflow">) => void;
  onDelete: (nodeId: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const metadata = getNodeMetadata(node.data.kind);
  const nodeType = capabilities.nodeTypes.find((candidate) => candidate.kind === node.data.kind);
  if (nodeType === undefined) {
    throw new Error(`Missing workflow capability for node kind "${node.data.kind}"`);
  }
  const Icon = metadata.icon;
  const agentConfig = node.data.agentConfig;
  return (
    <aside className="flex min-h-0 flex-1 flex-col border-l border-border bg-background">
      <div className="flex items-center gap-2.5 border-b border-border px-4 py-3">
        <span className={`flex size-8 items-center justify-center rounded-lg ${metadata.tone}`}>
          <Icon className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="text-xs font-semibold">{node.data.title}</h3>
          <p className="text-[10px] text-muted-foreground">
            {t("settings.workflow.nodeSuffix", { type: nodeType.label })}
          </p>
        </div>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("settings.workflow.closeConfiguration")}
          onClick={onClose}
        >
          <IconLayoutSidebarRightCollapse />
        </Button>
      </div>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
        <InspectorField label={t("settings.workflow.field.name")} htmlFor="workflow-node-title">
          <Input
            id="workflow-node-title"
            value={node.data.title}
            onChange={(event) => onUpdate({
              ...node,
              data: { ...node.data, title: event.target.value },
            })}
          />
        </InspectorField>
        <InspectorField label={t("settings.workflow.field.description")} htmlFor="workflow-node-description">
          <Input
            id="workflow-node-description"
            value={node.data.description}
            onChange={(event) => onUpdate({
              ...node,
              data: { ...node.data, description: event.target.value },
            })}
          />
        </InspectorField>
        {nodeType.configFields.includes("model") && (
          <InspectorField label={t("settings.workflow.field.model")} htmlFor="workflow-node-model">
            <Select
              value={node.data.model ?? capabilities.defaultModel}
              onValueChange={(model) => {
                if (model !== null) {
                  onUpdate({ ...node, data: { ...node.data, model } });
                }
              }}
            >
              <SelectTrigger id="workflow-node-model" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {capabilities.models.map((model) => (
                  <SelectItem key={model.value} value={model.value}>
                    {model.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </InspectorField>
        )}
        {nodeType.configFields.includes("tool") && (
          <InspectorField label={t("settings.workflow.field.tool")} htmlFor="workflow-node-tool">
            <Select
              value={node.data.tool ?? capabilities.defaultTool}
              onValueChange={(tool) => {
                if (tool !== null) {
                  onUpdate({ ...node, data: { ...node.data, tool } });
                }
              }}
            >
              <SelectTrigger id="workflow-node-tool" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {capabilities.tools.map((tool) => (
                  <SelectItem key={tool.value} value={tool.value}>
                    {tool.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </InspectorField>
        )}
        {nodeType.configFields.includes("condition") && (
          <InspectorField label={t("settings.workflow.field.condition")} htmlFor="workflow-node-condition">
            <Input
              id="workflow-node-condition"
              value={node.data.condition ?? ""}
              onChange={(event) =>
                onUpdate({
                  ...node,
                  data: { ...node.data, condition: event.target.value },
                })
              }
            />
          </InspectorField>
        )}
        {nodeType.configFields.includes("agent") && agentConfig !== undefined && (
          <AgentConfigurationFields
            config={agentConfig}
            capabilities={capabilities}
            modelsLoading={agentModelsLoading}
            onChange={(config) => onUpdate({
              ...node,
              data: { ...node.data, agentConfig: config },
            })}
          />
        )}
        {nodeType.configFields.includes("instruction") && (
          <InspectorField label={t("settings.workflow.field.instruction")} htmlFor="workflow-node-instruction">
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
  config,
  capabilities,
  modelsLoading,
  onChange,
}: {
  config: WorkflowAgentConfig;
  capabilities: WorkflowCapabilities;
  modelsLoading: boolean;
  onChange: (config: WorkflowAgentConfig) => void;
}) {
  const { t } = useTranslation();
  const [skillPickerOpen, setSkillPickerOpen] = useState(false);
  const configuredModel = capabilities.agentModels.find(
    (model) => model.agentCli === config.executor.agentCli
      && model.modelId === config.executor.modelId,
  );
  const selectedModel = configuredModel ?? {
    agentCli: config.executor.agentCli,
    modelId: config.executor.modelId,
    label: `${config.executor.agentCli} · ${config.executor.modelId}`,
  };
  const selectableModels = configuredModel === undefined
    ? [selectedModel, ...capabilities.agentModels]
    : capabilities.agentModels;
  const configuredSkillIds = new Set(config.skills.map((skill) => skill.skillId));
  const availableSkills = capabilities.skills.filter((skill) =>
    !configuredSkillIds.has(skill.value),
  );
  const enabledSkillCount = config.skills.filter((skill) => skill.enabled).length;
  const configuredRole = capabilities.roles.find((role) => role.value === config.roleId);
  const selectedRole = configuredRole ?? { value: config.roleId, label: config.roleId };
  const selectableRoles = configuredRole === undefined
    ? [selectedRole, ...capabilities.roles]
    : capabilities.roles;

  /** Replaces the selected model only after resolving a structured capability reference. */
  function selectModel(value: string | null): void {
    if (value === null) {
      return;
    }
    const model = selectableModels.find((candidate) => candidate.label === value);
    if (model === undefined) {
      return;
    }
    onChange({
      ...config,
      executor: { agentCli: model.agentCli, modelId: model.modelId },
    });
  }

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
        skill.skillId === skillId ? { ...skill, enabled } : skill),
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
      <InspectorField label={t("settings.workflow.field.agentModel")} htmlFor="workflow-agent-model">
        <Select value={selectedModel.label} onValueChange={selectModel}>
          <SelectTrigger
            id="workflow-agent-model"
            className="w-full"
            disabled={modelsLoading || capabilities.agentModels.length === 0}
          >
            <SelectValue placeholder={modelsLoading
              ? t("chat.modelSelector.loading")
              : t("chat.modelSelector.empty")}
            />
          </SelectTrigger>
          <SelectContent>
            {selectableModels.map((model) => (
              <SelectItem
                key={model.label}
                value={model.label}
              >
                {model.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </InspectorField>
      <InspectorField label={t("settings.workflow.field.role")} htmlFor="workflow-agent-role">
        <Select
          value={selectedRole.label}
          onValueChange={(label) => {
            if (label !== null) {
              const role = selectableRoles.find((candidate) => candidate.label === label);
              if (role !== undefined) {
                onChange({ ...config, roleId: role.value });
              }
            }
          }}
        >
          <SelectTrigger id="workflow-agent-role" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {selectableRoles.map((role) => (
              <SelectItem key={role.value} value={role.label}>{role.label}</SelectItem>
            ))}
          </SelectContent>
        </Select>
      </InspectorField>
      <fieldset className="space-y-2">
        <div className="flex items-center justify-between">
          <legend className="text-[11px] font-medium">
            {t("settings.workflow.field.skills")}
          </legend>
          <div className="flex items-center gap-1">
            <span className="text-[10px] text-muted-foreground">
              {t("settings.workflow.enabledSkillCount", {
                enabled: enabledSkillCount,
                total: config.skills.length,
              })}
            </span>
            <Popover open={skillPickerOpen} onOpenChange={setSkillPickerOpen}>
              <PopoverTrigger
                render={
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-sm"
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
                      {t("settings.workflow.noAvailableSkills")}
                    </CommandEmpty>
                    <CommandGroup>
                      {availableSkills.map((skill) => (
                        <CommandItem
                          key={skill.value}
                          value={skill.label}
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
        <div className="divide-y overflow-hidden rounded-md border border-border">
          {config.skills.map((configuredSkill) => {
            const skill = capabilities.skills.find(
              (candidate) => candidate.value === configuredSkill.skillId,
            ) ?? { value: configuredSkill.skillId, label: configuredSkill.skillId };
            return (
              <div key={configuredSkill.skillId} className="flex items-center gap-2 px-2.5 py-2">
                <span className="min-w-0 flex-1 truncate text-xs">{skill.label}</span>
                <Switch
                  size="sm"
                  className="data-checked:bg-blue-600 hover:data-checked:bg-blue-700"
                  checked={configuredSkill.enabled}
                  aria-label={t("settings.workflow.toggleSkill", { name: skill.label })}
                  onCheckedChange={(enabled) => setSkillEnabled(configuredSkill.skillId, enabled)}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                  aria-label={t("settings.workflow.removeSkill", { name: skill.label })}
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
      <InspectorField label={t("settings.workflow.field.prompt")} htmlFor="workflow-agent-prompt">
        <Textarea
          id="workflow-agent-prompt"
          className="min-h-32 resize-none text-xs leading-5"
          value={config.prompt}
          onChange={(event) => onChange({ ...config, prompt: event.target.value })}
        />
      </InspectorField>
    </>
  );
}

/** Keeps field labels visible and consistently spaced for scanning and accessibility. */
function InspectorField({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={htmlFor} className="text-[11px]">{label}</Label>
      {children}
    </div>
  );
}
