import { operatorSupportsType } from "./workflow-condition-operator";
import { selectorToText, textToSelector } from "./workflow-selector-text";
import { Fragment } from "react";
import { useTranslation } from "react-i18next";
import { IconPlus, IconTrash } from "@tabler/icons-react";
import type {
  WorkflowChoice,
  WorkflowConditionComparison,
  WorkflowConditionLogic,
  WorkflowVariableCatalogEntry,
} from "@ora/workflow-mock";
import {
  Button,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  cn,
} from "@ora/ui";
import { WorkflowVariableDisplay } from "./workflow-variable-display";
import { WorkflowVariableSelectGroups } from "./workflow-variable-list";
import { WorkflowPromptEditor } from "./workflow-prompt-editor";

/** Edits one condition group shared by branch routing and Loop termination. */
export function WorkflowConditionRules({
  conditions,
  logic,
  onChange,
  operators,
  variableCatalog,
  logicLabel,
  branchLabel,
  minimumConditions = 0,
}: {
  conditions: WorkflowConditionComparison[];
  logic: WorkflowConditionLogic;
  onChange: (group: {
    logic: WorkflowConditionLogic;
    conditions: WorkflowConditionComparison[];
  }) => void;
  operators: WorkflowChoice[];
  variableCatalog: WorkflowVariableCatalogEntry[];
  logicLabel: string;
  /** When set, drawn in the left rail like Dify's IF / ELIF label. */
  branchLabel?: string;
  minimumConditions?: number;
}) {
  const { t } = useTranslation();
  const logicOptions = [
    { value: "and", label: t("settings.workflow.condition.logicAnd") },
    { value: "or", label: t("settings.workflow.condition.logicOr") },
  ];
  function updateComparison(
    index: number,
    patch: Partial<WorkflowConditionComparison>,
  ): void {
    onChange({
      logic,
      conditions: conditions.map((item, candidate) =>
        candidate === index ? { ...item, ...patch } : item,
      ),
    });
  }
  function addComparison(): void {
    onChange({
      logic,
      conditions: [...conditions, { variableSelector: [], operator: "" }],
    });
  }
  function removeComparison(index: number): void {
    onChange({
      logic,
      conditions: conditions.filter((_, candidate) => candidate !== index),
    });
  }
  const showRail = branchLabel !== undefined || conditions.length > 1;
  return (
    <div
      className={cn(
        "grid min-w-0 items-stretch gap-x-1.5 gap-y-1.5 overflow-hidden",
        showRail ? "grid-cols-[2rem_minmax(0,1fr)]" : "grid-cols-1",
      )}
    >
      {conditions.map((comparison, comparisonIndex) => {
        const variableType = variableCatalog.find(
          (entry) =>
            selectorToText(entry.selector) ===
            selectorToText(comparison.variableSelector),
        )?.valueType;
        const availableOperators = operators.filter((operator) =>
          operatorSupportsType(operator.value, variableType),
        );
        const needsValue = ![
          "empty",
          "not_empty",
          "exists",
          "not_exists",
        ].includes(comparison.operator);
        return (
          <Fragment key={comparisonIndex}>
            {comparisonIndex > 0 && showRail && (
              <>
                <div className="relative">
                  <span className="absolute inset-y-[-0.35rem] left-1/2 w-px -translate-x-1/2 bg-border" />
                </div>
                <div className="flex min-w-0 items-center">
                  <Select
                    value={logic}
                    onValueChange={(nextLogic) => {
                      if (nextLogic === "and" || nextLogic === "or") {
                        onChange({ logic: nextLogic, conditions });
                      }
                    }}
                  >
                    <SelectTrigger
                      size="sm"
                      aria-label={`${logicLabel} · ${comparisonIndex}`}
                      className="h-5 min-h-5 w-auto shrink-0 justify-center gap-0 rounded-md border-blue-200 bg-background px-1.5 py-0 pr-1.5 text-[10px] font-semibold text-blue-600 shadow-sm dark:border-blue-800 dark:text-blue-400 [&_svg]:hidden"
                    >
                      <span>{logic.toUpperCase()}</span>
                    </SelectTrigger>
                    <SelectContent
                      alignItemWithTrigger={false}
                      side="bottom"
                      align="start"
                      className="w-max min-w-52 max-w-[min(18rem,calc(100vw-2rem))]"
                    >
                      {logicOptions.map((option) => (
                        <SelectItem key={option.value} value={option.value}>
                          {option.value.toUpperCase()} · {option.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </>
            )}
            {showRail && (
              <div className="relative flex justify-center">
                {conditions.length > 1 && (
                  <span className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-border" />
                )}
                {comparisonIndex === 0 && branchLabel !== undefined && (
                  <span className="relative z-10 mt-1.5 bg-background px-0.5 text-[11px] font-semibold leading-4">
                    {branchLabel}
                  </span>
                )}
              </div>
            )}
            <div className="min-w-0 overflow-hidden rounded-lg border border-border/60 bg-muted/40">
              <div className="flex min-w-0 items-center gap-0.5 px-1 py-0.5">
                <Select
                  value={selectorToText(comparison.variableSelector)}
                  onValueChange={(value) => {
                    if (value !== null) {
                      const nextType = variableCatalog.find(
                        (entry) => selectorToText(entry.selector) === value,
                      )?.valueType;
                      updateComparison(comparisonIndex, {
                        variableSelector: textToSelector(value),
                        ...(nextType !== variableType
                          ? { operator: "", value: undefined }
                          : {}),
                      });
                    }
                  }}
                >
                  <SelectTrigger
                    className="h-7 w-auto max-w-[min(100%,11.5rem)] min-w-0 shrink justify-start gap-0.5 overflow-hidden border-0 bg-transparent px-1 pr-0.5 shadow-none dark:bg-transparent [&_[data-slot=select-value]]:flex-none [&_svg]:size-3"
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
                    className="w-[min(17.5rem,calc(100vw-2rem))] min-w-0 max-w-[min(17.5rem,var(--available-width))]"
                  >
                    <WorkflowVariableSelectGroups
                      variables={variableCatalog}
                      globalVariablesLabel={t(
                        "settings.workflow.globalVariables",
                      )}
                    />
                  </SelectContent>
                </Select>
                <Select
                  key={selectorToText(comparison.variableSelector)}
                  value={comparison.operator}
                  onValueChange={(operator) => {
                    if (operator !== null) {
                      updateComparison(comparisonIndex, {
                        operator,
                        value: undefined,
                      });
                    }
                  }}
                >
                  <SelectTrigger
                    size="sm"
                    aria-label={t("settings.workflow.field.operator", {
                      index: comparisonIndex + 1,
                    })}
                    className="h-7 w-fit max-w-[6rem] shrink-0 rounded-md border-0 bg-transparent px-1 text-xs shadow-none [&_[data-slot=select-value]]:flex-none [&_svg]:size-3"
                  >
                    <LocalizedSelectValue
                      options={operators}
                      value={comparison.operator}
                      placeholder={t(
                        "settings.workflow.condition.operatorPlaceholder",
                      )}
                    />
                  </SelectTrigger>
                  <SelectContent
                    alignItemWithTrigger={false}
                    className="min-w-36 max-w-[min(18rem,var(--available-width))]"
                  >
                    {availableOperators.map((operator) => (
                      <SelectItem key={operator.value} value={operator.value}>
                        {operator.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  className="ml-auto size-7 shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                  disabled={conditions.length <= minimumConditions}
                  aria-label={t("settings.workflow.condition.removeRule")}
                  onClick={() => removeComparison(comparisonIndex)}
                >
                  <IconTrash className="size-3.5" />
                </Button>
              </div>
              {needsValue && (
                <div className="min-w-0 border-t border-border/70 px-1 py-0.5">
                  {variableType === "boolean" ? (
                    <Select
                      value={
                        comparison.value === undefined
                          ? ""
                          : String(comparison.value)
                      }
                      onValueChange={(value) => {
                        if (value === "true" || value === "false")
                          updateComparison(comparisonIndex, {
                            value: value === "true",
                          });
                      }}
                    >
                      <SelectTrigger
                        className="h-8 w-full min-w-0 border-0 bg-transparent px-1.5 shadow-none dark:bg-transparent"
                        aria-label={t("settings.workflow.field.value", {
                          index: comparisonIndex + 1,
                        })}
                      >
                        <SelectValue
                          placeholder={t(
                            "settings.workflow.condition.valuePlaceholder",
                          )}
                        />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="true">true</SelectItem>
                        <SelectItem value="false">false</SelectItem>
                      </SelectContent>
                    </Select>
                  ) : (
                    <WorkflowPromptEditor
                      key={`${selectorToText(comparison.variableSelector)}-${comparison.operator}-${comparisonIndex}`}
                      compact
                      value={comparisonValueToText(comparison.value)}
                      variableCatalog={variableCatalog}
                      ariaLabel={t("settings.workflow.field.value", {
                        index: comparisonIndex + 1,
                      })}
                      insertVariableLabel={t(
                        "settings.workflow.field.insertVariable",
                      )}
                      placeholder={t(
                        "settings.workflow.condition.valuePlaceholder",
                      )}
                      onChange={(text) =>
                        updateComparison(comparisonIndex, {
                          value:
                            text.trim() === ""
                              ? undefined
                              : variableType === "string" ||
                                  variableType === "secret" ||
                                  text.includes("{{#")
                                ? text
                                : parseComparisonValue(text),
                        })
                      }
                    />
                  )}
                </div>
              )}
            </div>
          </Fragment>
        );
      })}
      {showRail && conditions.length === 0 && (
        <span className="pt-1.5 text-center text-[11px] font-semibold">
          {branchLabel}
        </span>
      )}
      {showRail && conditions.length > 0 && <div />}
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="w-fit justify-start border border-border bg-background shadow-sm"
        onClick={addComparison}
      >
        <IconPlus />
        {t("settings.workflow.condition.addRule")}
      </Button>
    </div>
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
export function LocalizedSelectValue({
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

/** Coerces the comparison value's text form into a JSON-ish value for the backend. */
function parseComparisonValue(text: string): unknown {
  if (text.includes("{{#")) {
    return text;
  }
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
