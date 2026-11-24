import { WorkflowLoopLimitFields } from "./workflow-loop-limit-fields";
import { selectorToText, textToSelector } from "./workflow-selector-text";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconArrowDown,
  IconArrowUp,
  IconLayoutSidebarRightCollapse,
  IconPlus,
  IconTrash,
} from "@tabler/icons-react";
import {
  DEFAULT_ITERATION_MAX_ITERATIONS,
  resolveConditionCases,
  type WorkflowCapabilities,
  type WorkflowConditionCase,
  type WorkflowInputVariable,
  type WorkflowIterationConfig,
  type WorkflowNodeData,
  type WorkflowNodeType,
  type WorkflowVariableCatalogEntry,
  type WorkflowGlobalVariable,
  deriveWorkflowVariableCatalog,
  normalizeWorkflowGlobalVariables,
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
} from "@ora/ui";
import type { Node } from "@xyflow/react";
import { getNodeMetadata } from "./workflow-node-metadata";
import { WorkflowVariableDisplay } from "./workflow-variable-display";
import {
  WorkflowConditionRules,
  LocalizedSelectValue,
} from "./workflow-condition-rules";
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
  /** Whole-graph node list, for panels whose configuration reads graph structure. */
  graphNodes?: Node<WorkflowNodeData, "workflow">[];
  /** Workflow-wide declarations used when resolving graph-owned selector types. */
  globalVariables?: WorkflowGlobalVariable[];
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
  graphNodes,
  globalVariables,
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
    case "aggregator":
      return (
        <AggregatorNodeDetails
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
    case "loopExit":
      return (
        <div className="space-y-4 p-4">
          <WorkflowNodeDetailsHeader
            {...{ node, nodeType, onUpdate, onClose }}
          />
          <p className="text-xs leading-5 text-muted-foreground">
            {nodeType.description}
          </p>
        </div>
      );
    case "loop":
      return (
        <LoopNodeDetails
          {...{
            node,
            nodeType,
            capabilities,
            onUpdate,
            onClose,
            variableCatalog,
            graphNodes,
          }}
        />
      );
    case "iteration":
      return (
        <IterationNodeDetails
          {...{ node, nodeType, onUpdate, onClose, variableCatalog }}
          graphNodes={graphNodes ?? [node]}
          globalVariables={globalVariables ?? []}
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
  const removeCase = (caseIndex: number): void => {
    updateCases(cases.filter((_, index) => index !== caseIndex));
  };
  const addCase = (): void => {
    const nextSequence =
      cases.reduce((max, conditionCase) => {
        const number = /^case-(\d+)$/.exec(conditionCase.id);
        return Math.max(max, number === null ? 0 : Number(number[1]));
      }, 0) + 1;
    updateCases([...cases, defaultConditionCase(nextSequence)]);
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
        {cases.map((conditionCase, caseIndex) => (
          <section
            key={conditionCase.id}
            className="space-y-2 border-b border-border/70 pb-4"
          >
            {caseIndex > 0 && (
              <div className="flex justify-end">
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
              </div>
            )}
            <WorkflowConditionRules
              conditions={conditionCase.conditions}
              logic={conditionCase.logic ?? "and"}
              onChange={(group) => updateCase(caseIndex, group)}
              operators={capabilities.conditionOperators}
              variableCatalog={variableCatalog}
              logicLabel={t("settings.workflow.condition.branchLogic", {
                index: caseIndex + 1,
              })}
              branchLabel={caseIndex === 0 ? "IF" : "ELIF"}
            />
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

/**
 * Aggregator panel: an ordered list of candidate variables. Array order is the priority
 * contract, so the only ordering control is the explicit move buttons and no operation ever
 * reorders the list implicitly. Candidate types must agree for the graph to parse.
 */
function AggregatorNodeDetails({
  node,
  nodeType,
  onUpdate,
  onClose,
  variableCatalog,
}: WorkflowNodeDetailsLayoutProps) {
  const { t } = useTranslation();
  const selectors = node.data.aggregatorConfig?.variables ?? [];
  const updateSelectors = (next: string[][]): void => {
    onUpdate({
      ...node,
      data: { ...node.data, aggregatorConfig: { variables: next } },
    });
  };
  const setSelector = (index: number, selector: string[]): void => {
    updateSelectors(
      selectors.map((candidate, candidateIndex) =>
        candidateIndex === index ? selector : candidate,
      ),
    );
  };
  const removeSelector = (index: number): void => {
    updateSelectors(
      selectors.filter((_, candidateIndex) => candidateIndex !== index),
    );
  };
  const moveSelector = (index: number, offset: -1 | 1): void => {
    const target = index + offset;
    if (target < 0 || target >= selectors.length) {
      return;
    }
    const next = [...selectors];
    const [moved] = next.splice(index, 1);
    next.splice(target, 0, moved!);
    updateSelectors(next);
  };
  const selectorType = (selector: string[]): string | undefined =>
    variableCatalog.find(
      (entry) => selectorToText(entry.selector) === selectorToText(selector),
    )?.valueType;
  const assignedTypes = selectors
    .map(selectorType)
    .filter((valueType) => valueType !== undefined);
  const mixedTypes = new Set(assignedTypes).size > 1;
  return (
    <>
      <WorkflowNodeDetailsHeader
        node={node}
        nodeType={nodeType}
        onUpdate={onUpdate}
        onClose={onClose}
      />
      <WorkflowNodeBody>
        <p className="text-[11px] leading-5 text-muted-foreground">
          {t("settings.workflow.aggregation.description")}
        </p>
        {selectors.map((selector, index) => {
          const valueType = selectorType(selector);
          return (
            <div key={index} className="flex items-start gap-1.5">
              <div className="min-w-0 flex-1 space-y-1">
                <Select
                  value={selectorToText(selector)}
                  onValueChange={(value) => {
                    if (value !== null) {
                      setSelector(index, textToSelector(value));
                    }
                  }}
                >
                  <SelectTrigger
                    className="h-8 w-full bg-background"
                    aria-label={t(
                      "settings.workflow.aggregation.selectorLabel",
                      {
                        index: index + 1,
                      },
                    )}
                  >
                    <VariableSelectValue
                      catalog={variableCatalog}
                      selector={selector}
                      placeholder={t(
                        "settings.workflow.aggregation.variablePlaceholder",
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
                {valueType !== undefined && (
                  <span className="text-[10px] text-muted-foreground">
                    {valueType}
                  </span>
                )}
              </div>
              <div className="mt-0.5 flex shrink-0 flex-col gap-0.5">
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  className="size-6 text-muted-foreground"
                  aria-label={t("settings.workflow.aggregation.moveUp")}
                  disabled={index === 0}
                  onClick={() => moveSelector(index, -1)}
                >
                  <IconArrowUp className="size-3" />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  className="size-6 text-muted-foreground"
                  aria-label={t("settings.workflow.aggregation.moveDown")}
                  disabled={index === selectors.length - 1}
                  onClick={() => moveSelector(index, 1)}
                >
                  <IconArrowDown className="size-3" />
                </Button>
              </div>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                className="mt-1 shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                aria-label={t("settings.workflow.aggregation.removeSelector")}
                onClick={() => removeSelector(index)}
              >
                <IconTrash className="size-3.5" />
              </Button>
            </div>
          );
        })}
        {selectors.length === 0 && (
          <p className="text-[11px] leading-5 text-muted-foreground">
            {t("settings.workflow.aggregation.emptyHint")}
          </p>
        )}
        {mixedTypes && (
          <p className="text-[11px] leading-5 text-destructive">
            {t("settings.workflow.aggregation.typeMismatch")}
          </p>
        )}
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="w-full justify-center bg-muted/70 font-semibold"
          onClick={() => updateSelectors([...selectors, []])}
        >
          <IconPlus />
          {t("settings.workflow.aggregation.addSelector")}
        </Button>
        <div className="space-y-1 border-t border-border/70 pt-4">
          <p className="text-[11px] leading-5 text-muted-foreground">
            {t("settings.workflow.aggregation.outputHint")}
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

/** A fresh ELIF branch starts empty and exposes an explicit Add condition action. */
function defaultConditionCase(sequence: number): WorkflowConditionCase {
  return {
    id: `case-${sequence}`,
    logic: "and",
    conditions: [],
  };
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

/** Loop panel: bounded execution and the carried value fed back between rounds. */
function LoopNodeDetails({
  capabilities,
  variableCatalog,
  graphNodes = [],
  node,
  nodeType,
  onUpdate,
  onClose,
}: WorkflowNodeDetailsLayoutProps) {
  const { t } = useTranslation();
  const loopConfig = node.data.loopConfig;
  const childNodes = graphNodes.filter(
    (candidate) => candidate.parentId === node.id,
  );
  const conditionCatalog: WorkflowVariableCatalogEntry[] = [
    ...variableCatalog.filter((entry) => entry.sourceNodeId !== node.id),
    ...deriveWorkflowVariableCatalog(childNodes, [], undefined, []).map(
      (entry) => ({
        ...entry,
        sourceNodeTitle: childNodes.find(
          (child) => child.id === entry.sourceNodeId,
        )?.data.title,
      }),
    ),
    ...(loopConfig?.variables ?? []).map((variable) => ({
      selector: [node.id, variable.name],
      sourceNodeId: node.id,
      sourceNodeTitle: node.data.title,
      scope: "node" as const,
      variableName: variable.name,
      valueType: variable.valueType,
    })),
  ].filter(
    (entry, index, entries) =>
      entries.findIndex(
        (candidate) =>
          candidate.selector.join(".") === entry.selector.join("."),
      ) === index,
  );
  const carriedVariable = loopConfig?.variables[0];
  const initialValue =
    carriedVariable?.initial.kind === "constant"
      ? String(carriedVariable.initial.value ?? "")
      : "";
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
          label={t("settings.workflow.field.loopInitialValue")}
          htmlFor="workflow-node-loop-initial-value"
        >
          <Input
            id="workflow-node-loop-initial-value"
            value={initialValue}
            disabled={loopConfig === undefined || carriedVariable === undefined}
            onChange={(event) => {
              if (loopConfig === undefined || carriedVariable === undefined) {
                return;
              }
              onUpdate({
                ...node,
                data: {
                  ...node.data,
                  loopConfig: {
                    ...loopConfig,
                    variables: [
                      {
                        ...carriedVariable,
                        initial: {
                          kind: "constant",
                          value: event.target.value,
                        },
                      },
                      ...loopConfig.variables.slice(1),
                    ],
                  },
                },
              });
            }}
          />
        </InspectorField>
        {loopConfig !== undefined && (
          <section
            className="space-y-3"
            aria-label={t("settings.workflow.loop.until")}
          >
            <h3 className="text-xs font-semibold">
              {t("settings.workflow.loop.until")}
            </h3>
            <WorkflowConditionRules
              conditions={loopConfig.until.conditions}
              logic={loopConfig.until.logic}
              onChange={(until) =>
                onUpdate({
                  ...node,
                  data: { ...node.data, loopConfig: { ...loopConfig, until } },
                })
              }
              operators={capabilities.conditionOperators}
              variableCatalog={conditionCatalog}
              logicLabel={t("settings.workflow.loop.untilLogic")}
              minimumConditions={0}
            />
          </section>
        )}
        <WorkflowLoopLimitFields
          config={loopConfig}
          onChange={(config) =>
            onUpdate({ ...node, data: { ...node.data, loopConfig: config } })
          }
        />
        <p className="rounded-lg border border-border bg-muted/25 px-3 py-2 text-[11px] leading-5 text-muted-foreground">
          {loopConfig === undefined
            ? t("settings.workflow.loop.legacyUnsupported")
            : t("settings.workflow.loop.defaultBehavior")}
        </p>
      </WorkflowNodeBody>
    </>
  );
}

/** Edits one iteration node's foreach configuration: iterator source, collect target,
 * error strategy, and the safety ceiling (ADR "iteration composite runtime" D1/D2). */
function IterationNodeDetails({
  node,
  nodeType,
  onUpdate,
  onClose,
  variableCatalog,
  graphNodes,
  globalVariables,
}: Omit<WorkflowNodeDetailsLayoutProps, "capabilities"> & {
  graphNodes: Node<WorkflowNodeData, "workflow">[];
  globalVariables: WorkflowGlobalVariable[];
}) {
  const { t } = useTranslation();
  const config: WorkflowIterationConfig = node.data.iterationConfig ?? {
    iteratorSelector: [],
    collectSelector: [],
    errorStrategy: "fail",
    maxIterations: DEFAULT_ITERATION_MAX_ITERATIONS,
  };
  const updateConfig = (patch: Partial<WorkflowIterationConfig>): void => {
    onUpdate({
      ...node,
      data: { ...node.data, iterationConfig: { ...config, ...patch } },
    });
  };
  // Iterator choices: every visible array-typed variable (start inputs, globals, and the
  // exposed arrays of upstream iterations).
  const iteratorChoices = variableCatalog.filter((variable) =>
    variable.valueType.startsWith("array"),
  );
  // Collect choices are derived by the workflow-mock catalog owner, then restricted to direct
  // region members. This keeps output and structured-output declarations in one source of truth.
  const memberIds = new Set(
    graphNodes
      .filter((candidate) => candidate.parentId === node.id)
      .map((candidate) => candidate.id),
  );
  const collectChoices: WorkflowVariableCatalogEntry[] =
    deriveWorkflowVariableCatalog(
      graphNodes,
      [],
      undefined,
      normalizeWorkflowGlobalVariables(globalVariables),
    )
      .filter(
        (variable) =>
          memberIds.has(variable.sourceNodeId) &&
          variable.selector.length === 2 &&
          (variable.variableName === "output" ||
            variable.variableName === "structured_output"),
      )
      .map((variable) => ({
        ...variable,
        sourceNodeTitle:
          graphNodes.find((candidate) => candidate.id === variable.sourceNodeId)
            ?.data.title ?? variable.sourceNodeTitle,
      }));
  return (
    <>
      <WorkflowNodeDetailsHeader
        node={node}
        nodeType={nodeType}
        onUpdate={onUpdate}
        onClose={onClose}
      />
      <WorkflowNodeBody>
        <p className="text-xs leading-relaxed text-muted-foreground">
          {t("settings.workflow.iteration.hint")}
        </p>
        <IterationSelectorFields
          {...{ config, iteratorChoices, collectChoices, updateConfig }}
        />
        <InspectorField
          label={t("settings.workflow.field.iterationErrorStrategy")}
          htmlFor="workflow-node-iteration-error-strategy"
        >
          <Select
            value={config.errorStrategy}
            onValueChange={(value) => {
              if (value === "fail" || value === "continue") {
                updateConfig({ errorStrategy: value });
              }
            }}
          >
            <SelectTrigger
              className="h-8 w-full bg-background"
              id="workflow-node-iteration-error-strategy"
              aria-label={t("settings.workflow.field.iterationErrorStrategy")}
            >
              <SelectValue>
                {config.errorStrategy === "continue"
                  ? t("settings.workflow.iteration.continueStrategy")
                  : t("settings.workflow.iteration.failStrategy")}
              </SelectValue>
            </SelectTrigger>
            <SelectContent alignItemWithTrigger={false} align="start">
              <SelectItem value="fail">
                {t("settings.workflow.iteration.failStrategy")}
              </SelectItem>
              <SelectItem value="continue">
                {t("settings.workflow.iteration.continueStrategy")}
              </SelectItem>
            </SelectContent>
          </Select>
        </InspectorField>
        <InspectorField
          label={t("settings.workflow.field.maxIterations")}
          htmlFor="workflow-node-iteration-max"
        >
          <Input
            id="workflow-node-iteration-max"
            type="number"
            min={1}
            value={config.maxIterations}
            onChange={(event) => {
              const parsed = Number(event.target.value);
              if (
                event.target.value !== "" &&
                Number.isFinite(parsed) &&
                parsed >= 1
              ) {
                updateConfig({ maxIterations: Math.floor(parsed) });
              }
            }}
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

export function IterationSelectorFields({
  config,
  iteratorChoices,
  collectChoices,
  updateConfig,
}: {
  config: Pick<WorkflowIterationConfig, "iteratorSelector" | "collectSelector">;
  iteratorChoices: WorkflowVariableCatalogEntry[];
  collectChoices: WorkflowVariableCatalogEntry[];
  updateConfig: (
    patch: Partial<
      Pick<WorkflowIterationConfig, "iteratorSelector" | "collectSelector">
    >,
  ) => void;
}) {
  const { t } = useTranslation();
  const iteratorValue = config.iteratorSelector.join(".");
  const collectValue = config.collectSelector.join(".");
  const iteratorVariable = iteratorChoices.find(
    (variable) => variable.selector.join(".") === iteratorValue,
  );
  const collectVariable = collectChoices.find(
    (variable) => variable.selector.join(".") === collectValue,
  );
  const selectorSelect = (
    htmlId: string,
    value: string,
    placeholder: string,
    choices: WorkflowVariableCatalogEntry[],
    resolved: WorkflowVariableCatalogEntry | undefined,
    onPick: (selector: string[]) => void,
    ariaLabel: string,
  ) => (
    <Select
      value={value === "" ? null : value}
      onValueChange={(picked) => {
        if (picked !== null) {
          const choice = choices.find(
            (entry) => entry.selector.join(".") === picked,
          );
          if (choice) onPick(choice.selector);
        }
      }}
    >
      <SelectTrigger
        className="h-8 w-full bg-background"
        id={htmlId}
        aria-label={ariaLabel}
      >
        {resolved !== undefined ? (
          <SelectValue placeholder={placeholder}>
            <WorkflowVariableDisplay
              variable={resolved}
              nodeName={resolved.sourceNodeTitle ?? resolved.sourceNodeId}
            />
          </SelectValue>
        ) : value === "" ? (
          <SelectValue placeholder={placeholder} />
        ) : (
          <SelectValue placeholder={placeholder}>{value}</SelectValue>
        )}
      </SelectTrigger>
      <SelectContent
        alignItemWithTrigger={false}
        align="start"
        className="w-70 min-w-70 max-w-70"
      >
        <WorkflowVariableSelectGroups
          variables={choices}
          globalVariablesLabel={t("settings.workflow.globalVariables")}
        />
      </SelectContent>
    </Select>
  );

  return (
    <>
      <InspectorField
        label={t("settings.workflow.field.iterationIterator")}
        htmlFor="workflow-node-iteration-iterator"
      >
        {selectorSelect(
          "workflow-node-iteration-iterator",
          iteratorValue,
          t("settings.workflow.iteration.iteratorPlaceholder"),
          iteratorChoices,
          iteratorVariable,
          (selector) => updateConfig({ iteratorSelector: selector }),
          t("settings.workflow.field.iterationIterator"),
        )}
        {config.iteratorSelector.length === 0 && (
          <p className="text-xs text-amber-600 dark:text-amber-400">
            {t("settings.workflow.iteration.iteratorEmpty")}
          </p>
        )}
      </InspectorField>
      <InspectorField
        label={t("settings.workflow.field.iterationCollect")}
        htmlFor="workflow-node-iteration-collect"
      >
        {selectorSelect(
          "workflow-node-iteration-collect",
          collectValue,
          t("settings.workflow.iteration.collectPlaceholder"),
          collectChoices,
          collectVariable,
          (selector) => updateConfig({ collectSelector: selector }),
          t("settings.workflow.field.iterationCollect"),
        )}
        {config.collectSelector.length === 0 && (
          <p className="text-xs text-amber-600 dark:text-amber-400">
            {t("settings.workflow.iteration.collectEmpty")}
          </p>
        )}
      </InspectorField>
    </>
  );
}
