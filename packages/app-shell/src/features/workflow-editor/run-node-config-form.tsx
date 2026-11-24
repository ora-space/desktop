import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@ora/ui";
import {
  isWorkflowConditionComparisonComplete,
  type WorkflowAgentConfig,
  type WorkflowCapabilities,
  type WorkflowConditionCase,
  type WorkflowConditionComparison,
  type WorkflowLoopConfig,
  type WorkflowIterationConfig,
  type WorkflowVariableCatalogEntry,
} from "@ora/workflow-mock";
import { AgentExecutionFields } from "./workflow-agent-execution-fields";
import { operatorSupportsType } from "./workflow-condition-operator";
import { WorkflowConditionRules } from "./workflow-condition-rules";
import { WorkflowLoopLimitFields } from "./workflow-loop-limit-fields";
import { IterationSelectorFields } from "./workflow-node-details";

interface EditableConfig {
  agent: Pick<WorkflowAgentConfig, "executor" | "prompt">;
  condition: { cases: WorkflowConditionCase[] };
  loop: Pick<WorkflowLoopConfig, "until" | "maxIterations">;
  iteration: Pick<
    WorkflowIterationConfig,
    "iteratorSelector" | "collectSelector"
  >;
}
export type RunNodeConfig = {
  [K in keyof EditableConfig]: { kind: K; config: EditableConfig[K] };
}[keyof EditableConfig];
type PatchPath =
  | "/agentConfig/prompt"
  | "/agentConfig/executor/modelId"
  | `/cases/${number}/logic`
  | `/cases/${number}/conditions`
  | `/cases/${number}/conditions/${number}/value`
  | "/loopConfig/maxIterations"
  | "/loopConfig/until"
  | "/iterationConfig/iteratorSelector"
  | "/iterationConfig/collectSelector";

export interface RunNodeConfigEdit {
  runId: string;
  /** Physical node execution selected by the run page. */
  nodeRunId: string;
  baseRevision: number;
  patch: { op: "replace"; path: PatchPath; value: unknown }[];
}

export interface RunNodeConfigFormProps extends Pick<
  RunNodeConfigEdit,
  "runId" | "nodeRunId" | "baseRevision"
> {
  effective: RunNodeConfig;
  variables: WorkflowVariableCatalogEntry[];
  /** Outputs of direct members visible at the selected iteration's collection boundary. */
  memberOutputs?: WorkflowVariableCatalogEntry[];
  capabilities: Pick<
    WorkflowCapabilities,
    "agentModels" | "conditionOperators"
  >;
  onSubmit: (edit: RunNodeConfigEdit) => void;
  onCancel: () => void;
}

export function RunNodeConfigForm(props: RunNodeConfigFormProps) {
  return (
    <Draft
      key={JSON.stringify([
        props.runId,
        props.nodeRunId,
        props.baseRevision,
        props.effective.kind,
      ])}
      {...props}
    />
  );
}

function findVariable(
  catalog: WorkflowVariableCatalogEntry[],
  selector: string[],
) {
  return catalog.find(
    (entry) =>
      entry.selector.length === selector.length &&
      entry.selector.every((part, i) => part === selector[i]),
  );
}

function validReferences(
  value: unknown,
  catalog: WorkflowVariableCatalogEntry[],
) {
  if (typeof value !== "string") return true;
  return value
    .split("{{#")
    .slice(1)
    .every((tail) => {
      const end = tail.indexOf("#}}");
      return (
        end > 0 &&
        findVariable(catalog, tail.slice(0, end).split(".")) !== undefined
      );
    });
}

/** A sole token keeps its declared type; surrounding text renders a string. */
function validOperand(
  rule: WorkflowConditionComparison,
  source: WorkflowVariableCatalogEntry,
  variables: WorkflowVariableCatalogEntry[],
) {
  if (!validReferences(rule.value, variables)) return false;
  if (rule.value === undefined) return true;
  const token =
    typeof rule.value === "string" &&
    /^\{\{#([^{}#]+)#\}\}$/.exec(rule.value.trim());
  const type = token
    ? findVariable(variables, token[1]!.split("."))?.valueType
    : typeof rule.value;
  if (type === "any") return true;
  if (
    [
      "greater_than",
      "greater_than_or_equal",
      "less_than",
      "less_than_or_equal",
    ].includes(rule.operator)
  ) {
    return type === "number" || type === "integer";
  }
  if (
    ["starts_with", "ends_with"].includes(rule.operator) ||
    (["contains", "not_contains"].includes(rule.operator) &&
      ["string", "secret"].includes(source.valueType))
  ) {
    return type === "string" || type === "secret";
  }
  return true;
}

function Draft({
  runId,
  nodeRunId,
  baseRevision,
  effective,
  variables,
  memberOutputs = [],
  capabilities,
  onSubmit,
  onCancel,
}: RunNodeConfigFormProps) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(effective);
  const [attempted, setAttempted] = useState(false);
  const iteratorChoices = variables.filter((v) =>
    v.valueType.startsWith("array"),
  );
  const collectChoices = memberOutputs.filter(
    (v) =>
      v.selector.length === 2 &&
      ["output", "structured_output"].includes(v.selector[1]!),
  );
  function invalidRuleField(rules: WorkflowConditionComparison[]) {
    for (const [offset, rule] of rules.entries()) {
      const index = offset + 1;
      const variable = findVariable(variables, rule.variableSelector);
      if (!variable) return t("settings.workflow.field.variable", { index });
      const offered = capabilities.conditionOperators.some(
        (op) => op.value === rule.operator,
      );
      if (
        (!offered &&
          rule.operator !== "is" &&
          rule.operator !== "is_not" &&
          rule.operator !== "exists" &&
          rule.operator !== "not_exists") ||
        !operatorSupportsType(rule.operator, variable.valueType)
      ) {
        return t("settings.workflow.field.operator", { index });
      }
      if (
        !isWorkflowConditionComparisonComplete(rule) ||
        !validOperand(rule, variable, variables)
      ) {
        return t("settings.workflow.field.value", { index });
      }
    }
  }
  let fields;
  let invalidField: string | undefined;
  switch (draft.kind) {
    case "agent": {
      const config = draft.config;
      if (!validReferences(config.prompt, variables))
        invalidField = t("settings.workflow.field.prompt");
      if (
        effective.kind === "agent" &&
        JSON.stringify(config.executor) !==
          JSON.stringify(effective.config.executor) &&
        !capabilities.agentModels.some(
          (model) =>
            model.agentCli === config.executor.agentCli &&
            model.modelId === config.executor.modelId,
        )
      )
        invalidField = t("settings.workflow.field.agentModel");
      fields = (
        <AgentExecutionFields
          config={config}
          capabilities={capabilities}
          variableCatalog={variables}
          onChange={(patch) =>
            setDraft({ ...draft, config: { ...config, ...patch } })
          }
        />
      );
      break;
    }
    case "condition": {
      const { cases } = draft.config;
      fields = cases.map((item, index) => {
        const label = t("settings.workflow.condition.branchLogic", {
          index: index + 1,
        });
        const invalid = invalidRuleField(item.conditions);
        if (invalid) invalidField ??= [label, invalid].join(" · ");
        return (
          <WorkflowConditionRules
            key={item.id}
            branchLabel={index === 0 ? "IF" : "ELIF"}
            logicLabel={label}
            conditions={item.conditions}
            logic={item.logic ?? "and"}
            variableCatalog={variables}
            operators={capabilities.conditionOperators}
            onChange={(group) =>
              setDraft({
                ...draft,
                config: {
                  cases: cases.map((entry, i) =>
                    i === index ? { ...entry, ...group } : entry,
                  ),
                },
              })
            }
          />
        );
      });
      break;
    }
    case "loop": {
      const config = draft.config;
      const invalid = invalidRuleField(config.until.conditions);
      if (invalid)
        invalidField = [t("settings.workflow.loop.until"), invalid].join(" · ");
      if (
        !Number.isInteger(config.maxIterations) ||
        config.maxIterations < 1 ||
        config.maxIterations > 100
      )
        invalidField = t("settings.workflow.field.maxIterations");
      fields = (
        <>
          <section aria-label={t("settings.workflow.loop.until")}>
            <h3 className="text-xs font-semibold">
              {t("settings.workflow.loop.until")}
            </h3>
            <WorkflowConditionRules
              {...config.until}
              variableCatalog={variables}
              operators={capabilities.conditionOperators}
              logicLabel={t("settings.workflow.loop.untilLogic")}
              onChange={(until) =>
                setDraft({ ...draft, config: { ...config, until } })
              }
            />
          </section>
          <WorkflowLoopLimitFields
            config={config}
            showExhaustionPolicy={false}
            onChange={(next) => setDraft({ ...draft, config: next })}
          />
        </>
      );
      break;
    }
    case "iteration": {
      const config = draft.config;
      if (!findVariable(iteratorChoices, config.iteratorSelector))
        invalidField = t("settings.workflow.field.iterationIterator");
      else if (!findVariable(collectChoices, config.collectSelector))
        invalidField = t("settings.workflow.field.iterationCollect");
      fields = (
        <IterationSelectorFields
          config={config}
          iteratorChoices={iteratorChoices}
          collectChoices={collectChoices}
          updateConfig={(patch) =>
            setDraft({ ...draft, config: { ...config, ...patch } })
          }
        />
      );
      break;
    }
  }
  const patch = configPatch(effective, draft);
  return (
    <form
      className="space-y-4"
      onSubmit={(event) => {
        event.preventDefault();
        setAttempted(true);
        if (invalidField === undefined && patch.length)
          onSubmit({ runId, nodeRunId, baseRevision, patch });
      }}
    >
      {fields}
      {attempted && invalidField && (
        <p role="alert">
          {t("settings.workflow.debug.invalid", { field: invalidField })}
        </p>
      )}
      <Button
        type="button"
        variant="outline"
        onClick={() => {
          setDraft(effective);
          setAttempted(false);
          onCancel();
        }}
      >
        {t("common.cancel")}
      </Button>
      <Button type="submit" disabled={!patch.length}>
        {t("settings.workflow.debug.review")}
      </Button>
    </form>
  );
}

/** Compare only fields the run page owns. Paths are relative to normalized node data. */
function configPatch(
  base: RunNodeConfig,
  draft: RunNodeConfig,
): RunNodeConfigEdit["patch"] {
  const patch: RunNodeConfigEdit["patch"] = [];
  function replace(path: PatchPath, previous: unknown, next: unknown) {
    if (JSON.stringify(previous) !== JSON.stringify(next))
      patch.push({ op: "replace", path, value: next });
  }
  if (base.kind === "agent" && draft.kind === "agent") {
    replace("/agentConfig/prompt", base.config.prompt, draft.config.prompt);
    replace(
      "/agentConfig/executor/modelId",
      base.config.executor.modelId,
      draft.config.executor.modelId,
    );
  } else if (base.kind === "loop" && draft.kind === "loop") {
    replace("/loopConfig/until", base.config.until, draft.config.until);
    replace(
      "/loopConfig/maxIterations",
      base.config.maxIterations,
      draft.config.maxIterations,
    );
  } else if (base.kind === "iteration" && draft.kind === "iteration") {
    replace(
      "/iterationConfig/iteratorSelector",
      base.config.iteratorSelector,
      draft.config.iteratorSelector,
    );
    replace(
      "/iterationConfig/collectSelector",
      base.config.collectSelector,
      draft.config.collectSelector,
    );
  } else if (base.kind === "condition" && draft.kind === "condition") {
    draft.config.cases.forEach((entry, i) => {
      const previous = base.config.cases[i]!;
      replace(
        `/cases/${i}/logic`,
        previous.logic ?? "and",
        entry.logic ?? "and",
      );
      const conditions = entry.conditions;
      const valuesOnly =
        previous.conditions.length === conditions.length &&
        conditions.every((rule, j) => {
          const before = previous.conditions[j]!;
          return (
            before.operator === rule.operator &&
            JSON.stringify(before.variableSelector) ===
              JSON.stringify(rule.variableSelector) &&
            (before.value === undefined) === (rule.value === undefined)
          );
        });
      if (valuesOnly)
        conditions.forEach((rule, j) =>
          replace(
            `/cases/${i}/conditions/${j}/value`,
            previous.conditions[j]!.value,
            rule.value,
          ),
        );
      else replace(`/cases/${i}/conditions`, previous.conditions, conditions);
    });
  }
  return patch;
}
