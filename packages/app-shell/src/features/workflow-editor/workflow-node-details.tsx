import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconChevronDown,
  IconLayoutSidebarRightCollapse,
  IconPlus,
  IconTrash,
} from "@tabler/icons-react";
import {
  WORKFLOW_CONTEXT_VARIABLES,
  type WorkflowCapabilities,
  type WorkflowChoice,
  type WorkflowConditionBranch,
  type WorkflowConditionRule,
  type WorkflowInputVariable,
  type WorkflowNodeData,
  type WorkflowNodeType,
} from "@ora/workflow-mock";
import {
  Button,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Textarea,
  cn,
} from "@ora/ui";
import type { Node } from "@xyflow/react";
import { getNodeMetadata } from "./workflow-node-metadata";

const NODE_DESCRIPTION_MAX_LENGTH = 30;
const DEFAULT_CONDITION_OPERATOR = "equals";

interface WorkflowNodeDetailsLayoutProps {
  node: Node<WorkflowNodeData, "workflow">;
  nodeType: WorkflowNodeType;
  capabilities: WorkflowCapabilities;
  onUpdate: (node: Node<WorkflowNodeData, "workflow">) => void;
  onClose: () => void;
}

/**
 * Dify-style node editors: each kind gets a purpose-specific panel — prompt
 * pairs a model picker with a prompt editor and input variables, condition
 * renders IF/ELSE branch rules, tool pairs its picker with an operation and
 * key/value parameters, and start defines its inputs. Agent and output keep
 * their dedicated flat layout instead of using this shell.
 */
export function WorkflowNodeDetailsLayout({
  node,
  nodeType,
  capabilities,
  onUpdate,
  onClose,
}: WorkflowNodeDetailsLayoutProps) {
  switch (node.data.kind) {
    case "start":
      return (
        <StartNodeDetails
          node={node}
          nodeType={nodeType}
          capabilities={capabilities}
          onUpdate={onUpdate}
          onClose={onClose}
        />
      );
    case "condition":
      return (
        <ConditionNodeDetails
          node={node}
          nodeType={nodeType}
          capabilities={capabilities}
          onUpdate={onUpdate}
          onClose={onClose}
        />
      );
    case "tool":
      return (
        <ToolNodeDetails
          node={node}
          nodeType={nodeType}
          capabilities={capabilities}
          onUpdate={onUpdate}
          onClose={onClose}
        />
      );
    case "junction":
      return (
        <JunctionNodeDetails
          node={node}
          nodeType={nodeType}
          onUpdate={onUpdate}
          onClose={onClose}
        />
      );
    case "human":
      return (
        <HumanNodeDetails
          node={node}
          nodeType={nodeType}
          onUpdate={onUpdate}
          onClose={onClose}
        />
      );
    case "loop":
      return (
        <LoopNodeDetails
          node={node}
          nodeType={nodeType}
          onUpdate={onUpdate}
          onClose={onClose}
        />
      );
    case "subflow":
      return (
        <SubflowNodeDetails
          node={node}
          nodeType={nodeType}
          onUpdate={onUpdate}
          onClose={onClose}
        />
      );
    default:
      throw new Error(
        `Missing workflow detail layout for node kind "${node.data.kind}"`,
      );
  }
}

/** Shared panel header: the type label heads the panel with the close action. */
function WorkflowNodeDetailsHeader({
  node,
  nodeType,
  onClose,
}: {
  node: Node<WorkflowNodeData, "workflow">;
  nodeType: WorkflowNodeType;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const metadata = getNodeMetadata(node.data.kind);
  const Icon = metadata.icon;
  return (
    <header className="flex min-w-0 items-center gap-2.5 border-b border-border px-4 py-3">
      <span
        className={`flex size-8 shrink-0 items-center justify-center rounded-lg ${metadata.tone}`}
      >
        <Icon className="size-4" />
      </span>
      <h3 className="min-w-0 flex-1 truncate text-xs font-semibold">
        {t("settings.workflow.nodeSuffix", { type: nodeType.label })}
      </h3>
      <Button
        variant="ghost"
        size="icon-sm"
        className="shrink-0"
        aria-label={t("settings.workflow.closeConfiguration")}
        onClick={onClose}
      >
        <IconLayoutSidebarRightCollapse />
      </Button>
    </header>
  );
}

/** Scrollable field body shared by every kind-specific panel. */
function WorkflowNodeBody({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-0 min-w-0 flex-1 space-y-4 overflow-x-hidden overflow-y-auto p-4">
      {children}
    </div>
  );
}

/** Name and description fields shared by every kind-specific panel. */
function NodeIdentityFields({
  node,
  onUpdate,
}: {
  node: Node<WorkflowNodeData, "workflow">;
  onUpdate: (node: Node<WorkflowNodeData, "workflow">) => void;
}) {
  const { t } = useTranslation();
  return (
    <>
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
    </>
  );
}

/** Collapsible "Advanced settings" group shared by the model-driven node kinds. */
function AdvancedSettingsSection({ children }: { children: React.ReactNode }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <CollapsibleTrigger className="flex w-full items-center gap-1.5 rounded-md px-1 py-1 text-[11px] font-medium text-muted-foreground outline-none transition-colors hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring">
        {t("settings.workflow.section.advanced")}
        <IconChevronDown
          className={cn(
            "size-3.5 shrink-0 transition-transform duration-200 motion-reduce:transition-none",
            open && "rotate-180",
          )}
        />
      </CollapsibleTrigger>
      <CollapsibleContent className="space-y-3 pt-2">
        {children}
      </CollapsibleContent>
    </Collapsible>
  );
}

/** Optional mock-engine step duration, grouped under advanced settings. */
function MockStepMsField({
  node,
  onUpdate,
}: {
  node: Node<WorkflowNodeData, "workflow">;
  onUpdate: (node: Node<WorkflowNodeData, "workflow">) => void;
}) {
  const { t } = useTranslation();
  return (
    <InspectorField
      label={t("settings.workflow.field.mockStepMs")}
      htmlFor="workflow-node-mock-step"
    >
      <Input
        id="workflow-node-mock-step"
        type="number"
        min={0}
        value={node.data.mockStepMs ?? ""}
        onChange={(event) => {
          const parsed = Number(event.target.value);
          onUpdate({
            ...node,
            data: {
              ...node.data,
              mockStepMs:
                event.target.value !== "" && Number.isFinite(parsed)
                  ? parsed
                  : undefined,
            },
          });
        }}
      />
    </InspectorField>
  );
}

/** Start panel: trigger method, workflow input variables, and the available context variables. */
function StartNodeDetails({
  node,
  nodeType,
  capabilities,
  onUpdate,
  onClose,
}: WorkflowNodeDetailsLayoutProps) {
  const { t } = useTranslation();
  const inputVariables = node.data.inputVariables ?? [];
  const updateVariables = (variables: WorkflowInputVariable[]): void => {
    onUpdate({ ...node, data: { ...node.data, inputVariables: variables } });
  };
  return (
    <>
      <WorkflowNodeDetailsHeader
        node={node}
        nodeType={nodeType}
        onClose={onClose}
      />
      <WorkflowNodeBody>
        <NodeIdentityFields node={node} onUpdate={onUpdate} />
        <InspectorField
          label={t("settings.workflow.field.trigger")}
          htmlFor="workflow-node-trigger"
        >
          <Select
            value={node.data.trigger ?? capabilities.defaultTrigger}
            onValueChange={(trigger) => {
              if (trigger !== null) {
                onUpdate({ ...node, data: { ...node.data, trigger } });
              }
            }}
          >
            <SelectTrigger id="workflow-node-trigger" className="w-full">
              <LocalizedSelectValue
                options={capabilities.startTriggers}
                value={node.data.trigger ?? capabilities.defaultTrigger}
              />
            </SelectTrigger>
            <SelectContent>
              {capabilities.startTriggers.map((trigger) => (
                <SelectItem key={trigger.value} value={trigger.value}>
                  {trigger.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </InspectorField>
        <WorkflowNodeSection
          title={t("settings.workflow.section.inputVariables")}
        >
          <div className="space-y-2">
            {inputVariables.map((variable, index) => (
              <div
                key={index}
                className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] items-center gap-2"
              >
                <Input
                  value={variable.name}
                  aria-label={t("settings.workflow.field.inputVariableName", {
                    index: index + 1,
                  })}
                  placeholder={t(
                    "settings.workflow.field.inputVariableNamePlaceholder",
                  )}
                  className="h-8"
                  onChange={(event) =>
                    updateVariables(
                      inputVariables.map((candidate, candidateIndex) =>
                        candidateIndex === index
                          ? { ...candidate, name: event.target.value }
                          : candidate,
                      ),
                    )
                  }
                />
                <Input
                  value={variable.defaultValue ?? ""}
                  aria-label={t(
                    "settings.workflow.field.inputVariableDefault",
                    { index: index + 1 },
                  )}
                  placeholder={t("settings.workflow.field.defaultPlaceholder")}
                  className="h-8"
                  onChange={(event) =>
                    updateVariables(
                      inputVariables.map((candidate, candidateIndex) =>
                        candidateIndex === index
                          ? { ...candidate, defaultValue: event.target.value }
                          : candidate,
                      ),
                    )
                  }
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                  aria-label={t("settings.workflow.start.removeVariable")}
                  onClick={() =>
                    updateVariables(
                      inputVariables.filter(
                        (_, candidateIndex) => candidateIndex !== index,
                      ),
                    )
                  }
                >
                  <IconTrash className="size-3.5" />
                </Button>
              </div>
            ))}
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="w-full justify-start"
              onClick={() =>
                updateVariables([
                  ...inputVariables,
                  { name: "", defaultValue: "" },
                ])
              }
            >
              <IconPlus />
              {t("settings.workflow.start.addVariable")}
            </Button>
          </div>
        </WorkflowNodeSection>
        <WorkflowNodeSection
          title={t("settings.workflow.section.availableVariables")}
        >
          <p className="text-[10px] leading-4 text-muted-foreground">
            {t("settings.workflow.start.availableHint")}
          </p>
          <div className="flex flex-wrap gap-1.5">
            {WORKFLOW_CONTEXT_VARIABLES.map((name) => (
              <code
                key={name}
                className="rounded-md bg-muted px-1.5 py-0.5 font-mono text-[10px] text-foreground/80"
              >
                {name}
              </code>
            ))}
          </div>
        </WorkflowNodeSection>
        <AdvancedSettingsSection>
          <InspectorField
            label={t("settings.workflow.field.instruction")}
            htmlFor="workflow-node-instruction"
          >
            <Textarea
              id="workflow-node-instruction"
              className="min-h-24 resize-none text-xs leading-5"
              value={node.data.instruction ?? ""}
              onChange={(event) =>
                onUpdate({
                  ...node,
                  data: { ...node.data, instruction: event.target.value },
                })
              }
            />
          </InspectorField>
          <MockStepMsField node={node} onUpdate={onUpdate} />
        </AdvancedSettingsSection>
      </WorkflowNodeBody>
    </>
  );
}

/** IF/ELSE panel: branch cards with rule rows, plus the implicit default branch. */
function ConditionNodeDetails({
  node,
  nodeType,
  capabilities,
  onUpdate,
  onClose,
}: WorkflowNodeDetailsLayoutProps) {
  const { t } = useTranslation();
  const branches = node.data.conditionBranches ?? defaultConditionBranches();
  const updateBranches = (next: WorkflowConditionBranch[]): void => {
    onUpdate({ ...node, data: { ...node.data, conditionBranches: next } });
  };
  const updateBranch = (
    branchIndex: number,
    patch: Partial<WorkflowConditionBranch>,
  ): void => {
    updateBranches(
      branches.map((branch, candidateIndex) =>
        candidateIndex === branchIndex ? { ...branch, ...patch } : branch,
      ),
    );
  };
  const updateRule = (
    branchIndex: number,
    ruleIndex: number,
    patch: Partial<WorkflowConditionRule>,
  ): void => {
    updateBranches(
      branches.map((branch, candidateIndex) =>
        candidateIndex === branchIndex
          ? {
              ...branch,
              conditions: branch.conditions.map((rule, candidateRuleIndex) =>
                candidateRuleIndex === ruleIndex ? { ...rule, ...patch } : rule,
              ),
            }
          : branch,
      ),
    );
  };
  const logicOptions = [
    { value: "and" as const, label: t("settings.workflow.condition.logicAnd") },
    { value: "or" as const, label: t("settings.workflow.condition.logicOr") },
  ];
  return (
    <>
      <WorkflowNodeDetailsHeader
        node={node}
        nodeType={nodeType}
        onClose={onClose}
      />
      <WorkflowNodeBody>
        <NodeIdentityFields node={node} onUpdate={onUpdate} />
        {branches.map((branch, branchIndex) => (
          <div
            key={branchIndex}
            className="overflow-hidden rounded-lg border border-border"
          >
            <div className="space-y-2 border-b border-border bg-muted/25 px-3 py-2">
              <div className="flex items-center justify-between">
                <span className="text-[11px] font-semibold">
                  {t("settings.workflow.condition.branchTitle", {
                    index: branchIndex + 1,
                  })}
                </span>
                {branchIndex > 0 && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                    aria-label={t("settings.workflow.condition.removeBranch")}
                    onClick={() =>
                      updateBranches(
                        branches.filter(
                          (_, candidateIndex) => candidateIndex !== branchIndex,
                        ),
                      )
                    }
                  >
                    <IconTrash className="size-3.5" />
                  </Button>
                )}
              </div>
              <Select
                value={branch.logic ?? "and"}
                onValueChange={(logic) => {
                  if (logic === "and" || logic === "or") {
                    updateBranch(branchIndex, { logic });
                  }
                }}
              >
                <SelectTrigger
                  aria-label={t("settings.workflow.condition.branchLogic", {
                    index: branchIndex + 1,
                  })}
                  className="h-7 w-full text-[11px]"
                >
                  <LocalizedSelectValue
                    options={logicOptions}
                    value={branch.logic ?? "and"}
                  />
                </SelectTrigger>
                <SelectContent>
                  {logicOptions.map((logic) => (
                    <SelectItem key={logic.value} value={logic.value}>
                      {logic.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-3 p-3">
              {branch.conditions.map((rule, ruleIndex) => (
                <div
                  key={ruleIndex}
                  className="space-y-2 rounded-md border border-border/70 bg-background p-2.5"
                >
                  <RuleField label={t("settings.workflow.field.variable")}>
                    <Input
                      value={rule.variable}
                      aria-label={t("settings.workflow.field.variable", {
                        index: ruleIndex + 1,
                      })}
                      placeholder={t(
                        "settings.workflow.condition.variablePlaceholder",
                      )}
                      className="h-8"
                      onChange={(event) =>
                        updateRule(branchIndex, ruleIndex, {
                          variable: event.target.value,
                        })
                      }
                    />
                  </RuleField>
                  <RuleField label={t("settings.workflow.field.operator")}>
                    <Select
                      value={rule.operator}
                      onValueChange={(operator) => {
                        if (operator !== null) {
                          updateRule(branchIndex, ruleIndex, { operator });
                        }
                      }}
                    >
                      <SelectTrigger
                        aria-label={t("settings.workflow.field.operator", {
                          index: ruleIndex + 1,
                        })}
                        className="h-8 w-full"
                      >
                        <LocalizedSelectValue
                          options={capabilities.conditionOperators}
                          value={rule.operator}
                        />
                      </SelectTrigger>
                      <SelectContent>
                        {capabilities.conditionOperators.map((operator) => (
                          <SelectItem
                            key={operator.value}
                            value={operator.value}
                          >
                            {operator.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </RuleField>
                  <RuleField label={t("settings.workflow.field.value")}>
                    <>
                      <Input
                        value={rule.value}
                        aria-label={t("settings.workflow.field.value", {
                          index: ruleIndex + 1,
                        })}
                        placeholder={t(
                          "settings.workflow.condition.valuePlaceholder",
                        )}
                        className="h-8"
                        onChange={(event) =>
                          updateRule(branchIndex, ruleIndex, {
                            value: event.target.value,
                          })
                        }
                      />
                      <Button
                        type="button"
                        variant={rule.negated === true ? "secondary" : "ghost"}
                        size="sm"
                        className={cn(
                          "h-8 shrink-0 px-2 text-[10px] font-semibold",
                          rule.negated === true &&
                            "text-amber-700 dark:text-amber-400",
                        )}
                        aria-label={t("settings.workflow.condition.toggleNot", {
                          index: ruleIndex + 1,
                        })}
                        aria-pressed={rule.negated === true}
                        onClick={() =>
                          updateRule(branchIndex, ruleIndex, {
                            negated: rule.negated === true ? undefined : true,
                          })
                        }
                      >
                        {t("settings.workflow.condition.not")}
                      </Button>
                      {ruleIndex > 0 && (
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-sm"
                          className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                          aria-label={t(
                            "settings.workflow.condition.removeRule",
                          )}
                          onClick={() =>
                            updateBranches(
                              branches.map((candidateBranch, candidateIndex) =>
                                candidateIndex === branchIndex
                                  ? {
                                      ...candidateBranch,
                                      conditions:
                                        candidateBranch.conditions.filter(
                                          (_, candidateRuleIndex) =>
                                            candidateRuleIndex !== ruleIndex,
                                        ),
                                    }
                                  : candidateBranch,
                              ),
                            )
                          }
                        >
                          <IconTrash className="size-3.5" />
                        </Button>
                      )}
                    </>
                  </RuleField>
                </div>
              ))}
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="w-full justify-start"
                onClick={() =>
                  updateBranches(
                    branches.map((candidateBranch, candidateIndex) =>
                      candidateIndex === branchIndex
                        ? {
                            ...candidateBranch,
                            conditions: [
                              ...candidateBranch.conditions,
                              defaultConditionRule(),
                            ],
                          }
                        : candidateBranch,
                    ),
                  )
                }
              >
                <IconPlus />
                {t("settings.workflow.condition.addRule")}
              </Button>
            </div>
          </div>
        ))}
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="w-full justify-start"
          onClick={() =>
            updateBranches([
              ...branches,
              { conditions: [defaultConditionRule()] },
            ])
          }
        >
          <IconPlus />
          {t("settings.workflow.condition.addBranch")}
        </Button>
        <div className="flex items-center gap-2 rounded-lg border border-border bg-muted/25 px-3 py-2.5">
          <span className="text-[11px] font-medium">
            {t("settings.workflow.condition.otherCases")}
          </span>
          <span className="ml-auto text-[11px] text-muted-foreground">
            {t("settings.workflow.condition.defaultBranch")}
          </span>
        </div>
        <AdvancedSettingsSection>
          <InspectorField
            label={t("settings.workflow.field.instruction")}
            htmlFor="workflow-node-instruction"
          >
            <Textarea
              id="workflow-node-instruction"
              className="min-h-24 resize-none text-xs leading-5"
              value={node.data.instruction ?? ""}
              onChange={(event) =>
                onUpdate({
                  ...node,
                  data: { ...node.data, instruction: event.target.value },
                })
              }
            />
          </InspectorField>
          <MockStepMsField node={node} onUpdate={onUpdate} />
        </AdvancedSettingsSection>
      </WorkflowNodeBody>
    </>
  );
}

/** One labeled row inside a condition rule (variable, operator, or value). */
function RuleField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid grid-cols-[64px_minmax(0,1fr)] items-center gap-2">
      <span className="text-[11px] text-muted-foreground">{label}</span>
      <div className="flex min-w-0 items-center gap-2">{children}</div>
    </div>
  );
}

/**
 * Renders the selected choice's localized label. Base UI's value element shows
 * the raw value, which equals the label for simple catalogs but not for
 * operator/operation choices, so the label must be resolved explicitly.
 */
function LocalizedSelectValue({
  options,
  value,
}: {
  options: WorkflowChoice[];
  value: string;
}) {
  return (
    <SelectValue>
      {(selected) =>
        options.find((option) => option.value === (selected ?? value))?.label ??
        String(selected ?? value)
      }
    </SelectValue>
  );
}

/** A fresh condition branch with one empty rule, materialized on first edit. */
function defaultConditionBranches(): WorkflowConditionBranch[] {
  return [{ conditions: [defaultConditionRule()] }];
}

function defaultConditionRule(): WorkflowConditionRule {
  return { variable: "", operator: DEFAULT_CONDITION_OPERATOR, value: "" };
}

/** Tool-card panel: tool picker, derived operation, key/value parameters, and advanced settings. */
function ToolNodeDetails({
  node,
  nodeType,
  capabilities,
  onUpdate,
  onClose,
}: WorkflowNodeDetailsLayoutProps) {
  const { t } = useTranslation();
  const selectedTool = node.data.tool ?? capabilities.defaultTool;
  const operations = capabilities.toolOperations[selectedTool] ?? [];
  const toolParameters = node.data.toolParameters ?? [];
  const updateParameters = (parameters: typeof toolParameters): void => {
    onUpdate({ ...node, data: { ...node.data, toolParameters: parameters } });
  };
  return (
    <>
      <WorkflowNodeDetailsHeader
        node={node}
        nodeType={nodeType}
        onClose={onClose}
      />
      <WorkflowNodeBody>
        <NodeIdentityFields node={node} onUpdate={onUpdate} />
        <InspectorField
          label={t("settings.workflow.field.tool")}
          htmlFor="workflow-node-tool"
        >
          <Select
            value={selectedTool}
            onValueChange={(tool) => {
              if (tool !== null) {
                onUpdate({
                  ...node,
                  data: {
                    ...node.data,
                    tool,
                    // Switch to the first operation of the newly selected tool.
                    operation:
                      (capabilities.toolOperations[tool] ?? [])[0]?.value ??
                      undefined,
                  },
                });
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
        {operations.length > 0 ? (
          <InspectorField
            label={t("settings.workflow.field.operation")}
            htmlFor="workflow-node-operation"
          >
            <Select
              value={node.data.operation ?? operations[0]!.value}
              onValueChange={(operation) => {
                if (operation !== null) {
                  onUpdate({ ...node, data: { ...node.data, operation } });
                }
              }}
            >
              <SelectTrigger id="workflow-node-operation" className="w-full">
                <LocalizedSelectValue
                  options={operations}
                  value={node.data.operation ?? operations[0]!.value}
                />
              </SelectTrigger>
              <SelectContent>
                {operations.map((operation) => (
                  <SelectItem key={operation.value} value={operation.value}>
                    {operation.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </InspectorField>
        ) : (
          <p className="text-[11px] text-muted-foreground">
            {t("settings.workflow.tool.noOperations")}
          </p>
        )}
        <WorkflowNodeSection title={t("settings.workflow.section.parameters")}>
          <div className="space-y-2">
            {toolParameters.map((parameter, index) => (
              <div
                key={index}
                className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] items-center gap-2"
              >
                <Input
                  value={parameter.key}
                  aria-label={t("settings.workflow.field.parameterName", {
                    index: index + 1,
                  })}
                  placeholder={t("settings.workflow.field.parameterName")}
                  className="h-8"
                  onChange={(event) =>
                    updateParameters(
                      toolParameters.map((candidate, candidateIndex) =>
                        candidateIndex === index
                          ? { ...candidate, key: event.target.value }
                          : candidate,
                      ),
                    )
                  }
                />
                <Input
                  value={parameter.value}
                  aria-label={t("settings.workflow.field.parameterValue", {
                    index: index + 1,
                  })}
                  placeholder={t("settings.workflow.field.parameterValue")}
                  className="h-8"
                  onChange={(event) =>
                    updateParameters(
                      toolParameters.map((candidate, candidateIndex) =>
                        candidateIndex === index
                          ? { ...candidate, value: event.target.value }
                          : candidate,
                      ),
                    )
                  }
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                  aria-label={t("settings.workflow.tool.removeParameter")}
                  onClick={() =>
                    updateParameters(
                      toolParameters.filter(
                        (_, candidateIndex) => candidateIndex !== index,
                      ),
                    )
                  }
                >
                  <IconTrash className="size-3.5" />
                </Button>
              </div>
            ))}
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="w-full justify-start"
              onClick={() =>
                updateParameters([...toolParameters, { key: "", value: "" }])
              }
            >
              <IconPlus />
              {t("settings.workflow.tool.addParameter")}
            </Button>
          </div>
        </WorkflowNodeSection>
        <AdvancedSettingsSection>
          <InspectorField
            label={t("settings.workflow.field.instruction")}
            htmlFor="workflow-node-instruction"
          >
            <Textarea
              id="workflow-node-instruction"
              className="min-h-24 resize-none text-xs leading-5"
              value={node.data.instruction ?? ""}
              onChange={(event) =>
                onUpdate({
                  ...node,
                  data: { ...node.data, instruction: event.target.value },
                })
              }
            />
          </InspectorField>
          <MockStepMsField node={node} onUpdate={onUpdate} />
        </AdvancedSettingsSection>
      </WorkflowNodeBody>
    </>
  );
}

/** Merge panel: wait strategy (all/any/count) plus failure strategy for upstream branches. */
function JunctionNodeDetails({
  node,
  nodeType,
  onUpdate,
  onClose,
}: Omit<WorkflowNodeDetailsLayoutProps, "capabilities">) {
  const { t } = useTranslation();
  const waitStrategy = node.data.waitStrategy ?? "all";
  const waitOptions = [
    { value: "all" as const, label: t("settings.workflow.junction.waitAll") },
    { value: "any" as const, label: t("settings.workflow.junction.waitAny") },
    {
      value: "count" as const,
      label: t("settings.workflow.junction.waitCount"),
    },
  ];
  const failureOptions = [
    { value: "fail" as const, label: t("settings.workflow.junction.failFast") },
    {
      value: "continue" as const,
      label: t("settings.workflow.junction.collectResults"),
    },
  ];
  return (
    <>
      <WorkflowNodeDetailsHeader
        node={node}
        nodeType={nodeType}
        onClose={onClose}
      />
      <WorkflowNodeBody>
        <NodeIdentityFields node={node} onUpdate={onUpdate} />
        <InspectorField
          label={t("settings.workflow.field.waitStrategy")}
          htmlFor="workflow-node-wait-strategy"
        >
          <Select
            value={waitStrategy}
            onValueChange={(strategy) => {
              if (
                strategy === "all" ||
                strategy === "any" ||
                strategy === "count"
              ) {
                onUpdate({
                  ...node,
                  data: { ...node.data, waitStrategy: strategy },
                });
              }
            }}
          >
            <SelectTrigger id="workflow-node-wait-strategy" className="w-full">
              <LocalizedSelectValue
                options={waitOptions}
                value={waitStrategy}
              />
            </SelectTrigger>
            <SelectContent>
              {waitOptions.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </InspectorField>
        {waitStrategy === "count" && (
          <InspectorField
            label={t("settings.workflow.field.waitCount")}
            htmlFor="workflow-node-wait-count"
          >
            <Input
              id="workflow-node-wait-count"
              type="number"
              min={1}
              value={node.data.waitCount ?? 1}
              onChange={(event) => {
                const parsed = Number(event.target.value);
                onUpdate({
                  ...node,
                  data: {
                    ...node.data,
                    waitCount:
                      event.target.value !== "" && Number.isFinite(parsed)
                        ? parsed
                        : undefined,
                  },
                });
              }}
            />
          </InspectorField>
        )}
        <InspectorField
          label={t("settings.workflow.field.failureStrategy")}
          htmlFor="workflow-node-failure-strategy"
        >
          <Select
            value={node.data.failureStrategy ?? "fail"}
            onValueChange={(strategy) => {
              if (strategy === "fail" || strategy === "continue") {
                onUpdate({
                  ...node,
                  data: { ...node.data, failureStrategy: strategy },
                });
              }
            }}
          >
            <SelectTrigger
              id="workflow-node-failure-strategy"
              className="w-full"
            >
              <LocalizedSelectValue
                options={failureOptions}
                value={node.data.failureStrategy ?? "fail"}
              />
            </SelectTrigger>
            <SelectContent>
              {failureOptions.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </InspectorField>
        <AdvancedSettingsSection>
          <InspectorField
            label={t("settings.workflow.field.instruction")}
            htmlFor="workflow-node-instruction"
          >
            <Textarea
              id="workflow-node-instruction"
              className="min-h-24 resize-none text-xs leading-5"
              value={node.data.instruction ?? ""}
              onChange={(event) =>
                onUpdate({
                  ...node,
                  data: { ...node.data, instruction: event.target.value },
                })
              }
            />
          </InspectorField>
          <MockStepMsField node={node} onUpdate={onUpdate} />
        </AdvancedSettingsSection>
      </WorkflowNodeBody>
    </>
  );
}

/** Human-confirmation panel: the approval prompt is the instruction the reviewer sees. */
function HumanNodeDetails({
  node,
  nodeType,
  onUpdate,
  onClose,
}: Omit<WorkflowNodeDetailsLayoutProps, "capabilities">) {
  const { t } = useTranslation();
  return (
    <>
      <WorkflowNodeDetailsHeader
        node={node}
        nodeType={nodeType}
        onClose={onClose}
      />
      <WorkflowNodeBody>
        <NodeIdentityFields node={node} onUpdate={onUpdate} />
        <InspectorField
          label={t("settings.workflow.field.approvalPrompt")}
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
        <AdvancedSettingsSection>
          <MockStepMsField node={node} onUpdate={onUpdate} />
        </AdvancedSettingsSection>
      </WorkflowNodeBody>
    </>
  );
}

/** Loop panel: max attempts plus the exit condition that ends the loop early. */
function LoopNodeDetails({
  node,
  nodeType,
  onUpdate,
  onClose,
}: Omit<WorkflowNodeDetailsLayoutProps, "capabilities">) {
  const { t } = useTranslation();
  return (
    <>
      <WorkflowNodeDetailsHeader
        node={node}
        nodeType={nodeType}
        onClose={onClose}
      />
      <WorkflowNodeBody>
        <NodeIdentityFields node={node} onUpdate={onUpdate} />
        <InspectorField
          label={t("settings.workflow.field.maxAttempts")}
          htmlFor="workflow-node-max-attempts"
        >
          <Input
            id="workflow-node-max-attempts"
            type="number"
            min={1}
            value={node.data.maxAttempts ?? 3}
            onChange={(event) => {
              const parsed = Number(event.target.value);
              onUpdate({
                ...node,
                data: {
                  ...node.data,
                  maxAttempts:
                    event.target.value !== "" && Number.isFinite(parsed)
                      ? parsed
                      : undefined,
                },
              });
            }}
          />
        </InspectorField>
        <InspectorField
          label={t("settings.workflow.field.exitCondition")}
          htmlFor="workflow-node-exit-condition"
        >
          <Input
            id="workflow-node-exit-condition"
            value={node.data.exitCondition ?? ""}
            placeholder={t("settings.workflow.loop.exitConditionPlaceholder")}
            onChange={(event) =>
              onUpdate({
                ...node,
                data: { ...node.data, exitCondition: event.target.value },
              })
            }
          />
        </InspectorField>
        <AdvancedSettingsSection>
          <InspectorField
            label={t("settings.workflow.field.instruction")}
            htmlFor="workflow-node-instruction"
          >
            <Textarea
              id="workflow-node-instruction"
              className="min-h-24 resize-none text-xs leading-5"
              value={node.data.instruction ?? ""}
              onChange={(event) =>
                onUpdate({
                  ...node,
                  data: { ...node.data, instruction: event.target.value },
                })
              }
            />
          </InspectorField>
          <MockStepMsField node={node} onUpdate={onUpdate} />
        </AdvancedSettingsSection>
      </WorkflowNodeBody>
    </>
  );
}

/** Subflow panel: a placeholder reference until the V2 execution engine lands. */
function SubflowNodeDetails({
  node,
  nodeType,
  onUpdate,
  onClose,
}: Omit<WorkflowNodeDetailsLayoutProps, "capabilities">) {
  const { t } = useTranslation();
  return (
    <>
      <WorkflowNodeDetailsHeader
        node={node}
        nodeType={nodeType}
        onClose={onClose}
      />
      <WorkflowNodeBody>
        <NodeIdentityFields node={node} onUpdate={onUpdate} />
        <p className="rounded-lg border border-border bg-muted/25 px-3 py-2 text-[11px] leading-5 text-muted-foreground">
          {t("settings.workflow.subflow.hint")}
        </p>
        <AdvancedSettingsSection>
          <InspectorField
            label={t("settings.workflow.field.instruction")}
            htmlFor="workflow-node-instruction"
          >
            <Textarea
              id="workflow-node-instruction"
              className="min-h-24 resize-none text-xs leading-5"
              value={node.data.instruction ?? ""}
              onChange={(event) =>
                onUpdate({
                  ...node,
                  data: { ...node.data, instruction: event.target.value },
                })
              }
            />
          </InspectorField>
          <MockStepMsField node={node} onUpdate={onUpdate} />
        </AdvancedSettingsSection>
      </WorkflowNodeBody>
    </>
  );
}

/** Dify-style section: a small heading above a stacked group of fields. */
function WorkflowNodeSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-2.5">
      <h4 className="text-[11px] font-medium uppercase tracking-[0.04em] text-muted-foreground">
        {title}
      </h4>
      <div className="space-y-3">{children}</div>
    </section>
  );
}

/** Keeps field labels visible and consistently spaced for scanning and accessibility. */
export function InspectorField({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor: string;
  children: React.ReactNode;
}) {
  return (
    <div className="min-w-0 space-y-1.5">
      <Label htmlFor={htmlFor} className="text-[11px]">
        {label}
      </Label>
      {children}
    </div>
  );
}
