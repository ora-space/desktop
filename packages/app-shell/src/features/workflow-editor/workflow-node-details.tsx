import { Fragment, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconLayoutSidebarRightCollapse,
  IconPlus,
  IconTrash,
} from "@tabler/icons-react";
import {
  resolveConditionCases,
  type WorkflowCapabilities,
  type WorkflowChoice,
  type WorkflowConditionCase,
  type WorkflowConditionComparison,
  type WorkflowInputVariable,
  type WorkflowNodeData,
  type WorkflowNodeType,
  type WorkflowVariableCatalogEntry,
} from "@ora/workflow-mock";
import {
  Button,
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
import { WorkflowVariableDisplay } from "./workflow-variable-display";
import { WorkflowVariableSelectGroups } from "./workflow-variable-list";
import { WorkflowStartVariables } from "./workflow-start-variables";

const NODE_DESCRIPTION_MAX_LENGTH = 30;
interface WorkflowNodeDetailsLayoutProps {
  node: Node<WorkflowNodeData, "workflow">;
  nodeType: WorkflowNodeType;
  capabilities: WorkflowCapabilities;
  onUpdate: (node: Node<WorkflowNodeData, "workflow">) => void;
  onClose: () => void;
  variableCatalog: WorkflowVariableCatalogEntry[];
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
  variableCatalog,
}: WorkflowNodeDetailsLayoutProps) {
  switch (node.data.kind) {
    case "start":
      return (
        <StartNodeDetails
          node={node}
          nodeType={nodeType}
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
          variableCatalog={variableCatalog}
        />
      );
    case "tool":
      return (
        <ToolNodeDetails
          {...{
            node,
            nodeType,
            capabilities,
            onUpdate,
            onClose,
            variableCatalog,
          }}
        />
      );
    case "junction":
      return (
        <JunctionNodeDetails
          {...{ node, nodeType, onUpdate, onClose, variableCatalog }}
        />
      );
    case "human":
      return (
        <HumanNodeDetails
          {...{ node, nodeType, onUpdate, onClose, variableCatalog }}
        />
      );
    case "loop":
      return (
        <LoopNodeDetails
          {...{ node, nodeType, onUpdate, onClose, variableCatalog }}
        />
      );
    case "subflow":
      return (
        <SubflowNodeDetails
          {...{ node, nodeType, onUpdate, onClose, variableCatalog }}
        />
      );
    default:
      throw new Error(
        `Missing workflow detail layout for node kind "${node.data.kind}"`,
      );
  }
}

/** Keeps node identity close to the panel title while leaving the body for behavior settings. */
export function WorkflowNodeDetailsHeader({
  node,
  nodeType,
  onUpdate,
  onClose,
}: {
  node: Node<WorkflowNodeData, "workflow">;
  nodeType: WorkflowNodeType;
  onUpdate: (node: Node<WorkflowNodeData, "workflow">) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState(node.data.title);
  const metadata = getNodeMetadata(node.data.kind);
  const Icon = metadata.icon;

  /** Commits a non-empty title and restores the existing title for blank edits. */
  function commitTitle(): void {
    const title = titleDraft.trim() || node.data.title;
    setTitleDraft(title);
    setEditingTitle(false);
    if (title !== node.data.title) {
      onUpdate({ ...node, data: { ...node.data, title } });
    }
  }

  return (
    <header className="min-w-0 border-b border-border px-4 py-3">
      <div className="flex min-w-0 items-center gap-2.5">
        <span
          className={`flex size-8 shrink-0 items-center justify-center rounded-lg ${metadata.tone}`}
          title={nodeType.label}
        >
          <Icon className="size-4" />
        </span>
        {editingTitle ? (
          <Input
            autoFocus
            aria-label={t("settings.workflow.field.name")}
            className="h-9 min-w-0 flex-1 px-2 font-sans text-base font-bold"
            value={titleDraft}
            onChange={(event) => setTitleDraft(event.target.value)}
            onBlur={commitTitle}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.currentTarget.blur();
              } else if (event.key === "Escape") {
                setTitleDraft(node.data.title);
                setEditingTitle(false);
              }
            }}
          />
        ) : (
          <h3 className="min-w-0 flex-1 font-sans text-base font-bold">
            <button
              type="button"
              className="block w-full truncate rounded-sm text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
              title={t("settings.workflow.editNodeTitle")}
              onDoubleClick={() => {
                setTitleDraft(node.data.title);
                setEditingTitle(true);
              }}
            >
              {node.data.title}
            </button>
          </h3>
        )}
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
      <div className="mt-1 flex min-w-0 items-center gap-2">
        <Input
          aria-label={t("settings.workflow.field.description")}
          className="h-7 min-w-0 flex-1 border-0 bg-transparent px-0 text-[11px] text-muted-foreground shadow-none focus-visible:ring-0"
          placeholder={t("settings.workflow.addDescription")}
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
        <span className="shrink-0 text-[10px] text-muted-foreground">
          {t("settings.workflow.characterCount", {
            count: node.data.description.length,
            max: NODE_DESCRIPTION_MAX_LENGTH,
          })}
        </span>
      </div>
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

/** Start panel: workflow input variables and the prompt that begins the first Agent turn. */
function StartNodeDetails({
  node,
  nodeType,
  onUpdate,
  onClose,
}: Omit<WorkflowNodeDetailsLayoutProps, "capabilities" | "variableCatalog">) {
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
        onUpdate={onUpdate}
        onClose={onClose}
      />
      <WorkflowNodeBody>
        <WorkflowStartVariables
          variables={inputVariables}
          onChange={updateVariables}
        />
        <InspectorField
          label={t("settings.workflow.field.initialPrompt")}
          htmlFor="workflow-node-initial-prompt"
        >
          <Textarea
            id="workflow-node-initial-prompt"
            className="min-h-24 resize-none text-xs leading-5"
            value={node.data.input ?? ""}
            onChange={(event) =>
              onUpdate({
                ...node,
                data: { ...node.data, input: event.target.value },
              })
            }
          />
        </InspectorField>
      </WorkflowNodeBody>
    </>
  );
}

/** Executable IF/ELSE panel: case cards with comparisons, plus the implicit else branch. */
function ConditionNodeDetails({
  node,
  nodeType,
  capabilities,
  onUpdate,
  onClose,
  variableCatalog,
}: WorkflowNodeDetailsLayoutProps) {
  const { t } = useTranslation();
  const cases = resolveConditionCases(node.data);
  const updateCases = (next: WorkflowConditionCase[]): void => {
    const data = { ...node.data, cases: next };
    // Editing is the migration boundary: keep old saved graphs readable, then persist one shape.
    delete data.conditionCases;
    delete data.conditionBranches;
    onUpdate({ ...node, data });
  };
  const updateCase = (
    caseIndex: number,
    patch: Partial<WorkflowConditionCase>,
  ): void => {
    updateCases(
      cases.map((conditionCase, candidateIndex) =>
        candidateIndex === caseIndex
          ? { ...conditionCase, ...patch }
          : conditionCase,
      ),
    );
  };
  const updateComparison = (
    caseIndex: number,
    comparisonIndex: number,
    patch: Partial<WorkflowConditionComparison>,
  ): void => {
    updateCases(
      cases.map((conditionCase, candidateIndex) =>
        candidateIndex === caseIndex
          ? {
              ...conditionCase,
              conditions: conditionCase.conditions.map(
                (comparison, candidateComparisonIndex) =>
                  candidateComparisonIndex === comparisonIndex
                    ? { ...comparison, ...patch }
                    : comparison,
              ),
            }
          : conditionCase,
      ),
    );
  };
  const removeCase = (caseIndex: number): void => {
    updateCases(cases.filter((_, index) => index !== caseIndex));
  };
  const addComparison = (caseIndex: number): void => {
    updateCases(
      cases.map((conditionCase, candidateIndex) =>
        candidateIndex === caseIndex
          ? {
              ...conditionCase,
              conditions: [
                ...conditionCase.conditions,
                defaultConditionComparison(),
              ],
            }
          : conditionCase,
      ),
    );
  };
  const removeComparison = (
    caseIndex: number,
    comparisonIndex: number,
  ): void => {
    updateCases(
      cases.map((conditionCase, candidateIndex) =>
        candidateIndex === caseIndex
          ? {
              ...conditionCase,
              conditions: conditionCase.conditions.filter(
                (_, index) => index !== comparisonIndex,
              ),
            }
          : conditionCase,
      ),
    );
  };
  const addCase = (): void => {
    const nextSequence =
      cases.reduce((max, conditionCase) => {
        const number = /^case-(\d+)$/.exec(conditionCase.id);
        return Math.max(max, number === null ? 0 : Number(number[1]));
      }, 0) + 1;
    updateCases([...cases, defaultConditionCase(nextSequence)]);
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
        onUpdate={onUpdate}
        onClose={onClose}
      />
      <WorkflowNodeBody>
        {cases.map((conditionCase, caseIndex) => (
          <section
            key={conditionCase.id}
            className="space-y-2 border-b border-border/70 pb-4"
          >
            <div className="flex items-center justify-between">
              <span className="text-[11px] font-semibold">
                {caseIndex === 0 ? "IF" : "ELIF"}
              </span>
              <div className="flex items-center gap-1">
                {conditionCase.conditions.length === 0 && (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-7 bg-background shadow-sm"
                    onClick={() => addComparison(caseIndex)}
                  >
                    <IconPlus />
                    {t("settings.workflow.condition.addRule")}
                  </Button>
                )}
                {caseIndex > 0 && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                    aria-label={t("settings.workflow.condition.removeBranch")}
                    onClick={() => removeCase(caseIndex)}
                  >
                    <IconTrash className="size-3.5" />
                  </Button>
                )}
              </div>
            </div>
            <div
              className={cn(
                "px-1",
                conditionCase.conditions.length > 1 &&
                  "relative ml-2 border-l border-border/80 pl-3",
              )}
            >
              {conditionCase.conditions.map((comparison, comparisonIndex) => (
                <Fragment key={comparisonIndex}>
                  {comparisonIndex > 0 && (
                    <div className="relative h-7">
                      <Select
                        value={conditionCase.logic ?? "and"}
                        onValueChange={(logic) => {
                          if (logic === "and" || logic === "or") {
                            updateCase(caseIndex, { logic });
                          }
                        }}
                      >
                        <SelectTrigger
                          aria-label={`${t(
                            "settings.workflow.condition.branchLogic",
                            { index: caseIndex + 1 },
                          )} · ${comparisonIndex}`}
                          className="absolute -left-6 top-1/2 h-6 w-auto min-w-10 -translate-y-1/2 justify-center gap-1 rounded-md border-blue-200 bg-background px-1.5 text-[10px] font-semibold text-blue-600 shadow-sm dark:border-blue-800 dark:text-blue-400"
                        >
                          <span>
                            {(conditionCase.logic ?? "and").toUpperCase()}
                          </span>
                        </SelectTrigger>
                        <SelectContent>
                          {logicOptions.map((logic) => (
                            <SelectItem key={logic.value} value={logic.value}>
                              {logic.value.toUpperCase()} · {logic.label}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                  )}
                  <div className="flex items-start gap-1.5">
                    <div className="min-w-0 flex-1 space-y-1.5 rounded-lg bg-muted/70 p-2">
                      <Select
                        value={selectorToText(comparison.variableSelector)}
                        onValueChange={(value) => {
                          if (value !== null) {
                            updateComparison(caseIndex, comparisonIndex, {
                              variableSelector: textToSelector(value),
                            });
                          }
                        }}
                      >
                        <SelectTrigger
                          className="h-8 w-full bg-background"
                          aria-label={t("settings.workflow.field.variable", {
                            index: comparisonIndex + 1,
                          })}
                        >
                          <VariableSelectValue
                            catalog={variableCatalog}
                            selector={comparison.variableSelector}
                            placeholder={t(
                              "settings.workflow.condition.variablePlaceholder",
                            )}
                          />
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
                      <div className="flex min-w-0 gap-1.5">
                        <Select
                          value={comparison.operator}
                          onValueChange={(operator) => {
                            if (operator !== null) {
                              updateComparison(caseIndex, comparisonIndex, {
                                operator,
                              });
                            }
                          }}
                        >
                          <SelectTrigger
                            aria-label={t("settings.workflow.field.operator", {
                              index: comparisonIndex + 1,
                            })}
                            className="h-8 w-20 shrink-0 bg-background"
                          >
                            <LocalizedSelectValue
                              options={capabilities.conditionOperators}
                              value={comparison.operator}
                              placeholder={t(
                                "settings.workflow.condition.operatorPlaceholder",
                              )}
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
                        <Input
                          value={comparisonValueToText(comparison.value)}
                          aria-label={t("settings.workflow.field.value", {
                            index: comparisonIndex + 1,
                          })}
                          placeholder={t(
                            "settings.workflow.condition.valuePlaceholder",
                          )}
                          className="h-8 min-w-0 flex-1 bg-background"
                          onChange={(event) =>
                            updateComparison(caseIndex, comparisonIndex, {
                              value: parseComparisonValue(event.target.value),
                            })
                          }
                        />
                      </div>
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      className="mt-1 shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                      aria-label={t("settings.workflow.condition.removeRule")}
                      onClick={() =>
                        removeComparison(caseIndex, comparisonIndex)
                      }
                    >
                      <IconTrash className="size-3.5" />
                    </Button>
                  </div>
                </Fragment>
              ))}
              {conditionCase.conditions.length > 0 && (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="mt-3 w-fit justify-start border border-border bg-background shadow-sm"
                  onClick={() => addComparison(caseIndex)}
                >
                  <IconPlus />
                  {t("settings.workflow.condition.addRule")}
                </Button>
              )}
            </div>
          </section>
        ))}
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="w-full justify-center bg-muted/70 font-semibold"
          onClick={addCase}
        >
          <IconPlus />
          {t("settings.workflow.condition.addElif")}
        </Button>
        <div className="space-y-1 border-t border-border/70 pt-4">
          <span className="text-[11px] font-medium">ELSE</span>
          <p className="text-[11px] leading-5 text-muted-foreground">
            {t("settings.workflow.condition.elseDescription")}
          </p>
        </div>
      </WorkflowNodeBody>
    </>
  );
}

/** Renders the chosen variable with its node identity, or the selector text as a fallback. */
function VariableSelectValue({
  catalog,
  selector,
  placeholder,
}: {
  catalog: WorkflowVariableCatalogEntry[];
  selector: string[];
  placeholder: string;
}) {
  const { t } = useTranslation();
  const selectorText = selectorToText(selector);
  if (selectorText === "") {
    return <SelectValue placeholder={placeholder} />;
  }
  const variable = catalog.find(
    (candidate) => selectorToText(candidate.selector) === selectorText,
  );
  return (
    <SelectValue placeholder={placeholder}>
      {variable === undefined ? (
        selectorText
      ) : (
        <WorkflowVariableDisplay
          variable={variable}
          nodeName={
            variable.sourceNodeTitle ??
            (variable.scope === "global"
              ? t("settings.workflow.globalVariables")
              : variable.sourceNodeId)
          }
        />
      )}
    </SelectValue>
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
  placeholder,
}: {
  options: WorkflowChoice[];
  value: string;
  placeholder?: string;
}) {
  if (value === "" && placeholder !== undefined) {
    return <SelectValue placeholder={placeholder} />;
  }
  return (
    <SelectValue placeholder={placeholder}>
      {(selected) =>
        options.find((option) => option.value === (selected ?? value))?.label ??
        String(selected ?? value)
      }
    </SelectValue>
  );
}

/** A fresh ELIF branch starts empty and exposes an explicit Add condition action. */
function defaultConditionCase(sequence: number): WorkflowConditionCase {
  return {
    id: `case-${sequence}`,
    logic: "and",
    conditions: [],
  };
}

function defaultConditionComparison(): WorkflowConditionComparison {
  return { variableSelector: [], operator: "" };
}

/** Joins a selector array into its dotted text form for the editor input. */
function selectorToText(selector: string[]): string {
  return selector.join(".");
}

/** Splits the dotted selector text into `[nodeId, root, ...nested]` parts. */
function textToSelector(text: string): string[] {
  return text
    .split(".")
    .map((part) => part.trim())
    .filter((part) => part !== "");
}

/** Coerces the comparison value's text form into a JSON-ish value for the backend. */
function parseComparisonValue(text: string): unknown {
  const trimmed = text.trim();
  if (trimmed === "true") {
    return true;
  }
  if (trimmed === "false") {
    return false;
  }
  if (trimmed !== "" && Number.isFinite(Number(trimmed))) {
    return Number(trimmed);
  }
  return text;
}

/** Renders a comparison value back into its editable text form. */
function comparisonValueToText(value: unknown): string {
  if (value === undefined || value === null) {
    return "";
  }
  return typeof value === "string" ? value : JSON.stringify(value);
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
        onUpdate={onUpdate}
        onClose={onClose}
      />
      <WorkflowNodeBody>
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
        onUpdate={onUpdate}
        onClose={onClose}
      />
      <WorkflowNodeBody>
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
        onUpdate={onUpdate}
        onClose={onClose}
      />
      <WorkflowNodeBody>
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
        onUpdate={onUpdate}
        onClose={onClose}
      />
      <WorkflowNodeBody>
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
        onUpdate={onUpdate}
        onClose={onClose}
      />
      <WorkflowNodeBody>
        <p className="rounded-lg border border-border bg-muted/25 px-3 py-2 text-[11px] leading-5 text-muted-foreground">
          {t("settings.workflow.subflow.hint")}
        </p>
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
