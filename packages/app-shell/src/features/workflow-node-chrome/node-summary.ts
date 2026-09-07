import {
  createMockWorkflowCapabilities,
  resolveConditionCases,
  type WorkflowNodeData,
} from "@ora/workflow-mock";

/** Resolves stable label strings for structured node fields on read-only surfaces. */
export interface WorkflowSummaryLabels {
  operatorLabel: (operator: string) => string;
  operationLabel: (operation: string) => string;
}

/** Builds label resolvers from the localized mock capability catalogs. */
export function createWorkflowSummaryLabels(
  locale: "zh-CN" | "en-US",
): WorkflowSummaryLabels {
  const capabilities = createMockWorkflowCapabilities(locale);
  return {
    operatorLabel: (operator) =>
      capabilities.conditionOperators.find(
        (candidate) => candidate.value === operator,
      )?.label ?? operator,
    operationLabel: (operation) => {
      for (const operations of Object.values(capabilities.toolOperations)) {
        const found = operations.find(
          (candidate) => candidate.value === operation,
        );
        if (found !== undefined) {
          return found.label;
        }
      }
      return operation;
    },
  };
}

/** Localizes a junction wait strategy ("all" | "any" | "count"). */
export function junctionWaitStrategyLabel(
  strategy: string | undefined,
  t: (key: string) => string,
): string {
  if (strategy === "any") {
    return t("settings.workflow.junction.waitAny");
  }
  if (strategy === "count") {
    return t("settings.workflow.junction.waitCount");
  }
  return t("settings.workflow.junction.waitAll");
}

/** Localizes a junction failure strategy ("fail" | "continue"). */
export function junctionFailureStrategyLabel(
  strategy: string | undefined,
  t: (key: string) => string,
): string {
  return strategy === "continue"
    ? t("settings.workflow.junction.collectResults")
    : t("settings.workflow.junction.failFast");
}

/**
 * Compacts executable condition cases into one readable line, e.g.
 * `分支 1: 工具1.exit_code 等于 0`. Comparisons combine with `且`/`或` per the
 * case logic. Falls back to the legacy flat condition string so graphs saved
 * before structured cases still summarize correctly.
 */
export function conditionBranchesSummary(
  data: WorkflowNodeData,
  labels: WorkflowSummaryLabels,
  locale: "zh-CN" | "en-US",
): string | null {
  const cases = resolveConditionCases(data);
  if (cases.length === 0) {
    return data.condition ?? null;
  }
  const english = locale === "en-US";
  const and = english ? " and " : " 且 ";
  const or = english ? " or " : " 或 ";
  const lines = cases.map((conditionCase) =>
    conditionCase.conditions
      .map((comparison) =>
        [
          comparison.variableSelector.join("."),
          labels.operatorLabel(comparison.operator),
          comparisonValueLabel(comparison.value),
        ]
          .filter((part) => part !== "")
          .join(" "),
      )
      .filter((line) => line !== "")
      .join(conditionCase.logic === "or" ? or : and),
  );
  return lines.filter((line) => line !== "").join("；");
}

/** Renders a comparison value as text, JSON for non-strings. */
function comparisonValueLabel(value: unknown): string {
  if (value === undefined || value === null) {
    return "";
  }
  return typeof value === "string" ? value : JSON.stringify(value);
}
