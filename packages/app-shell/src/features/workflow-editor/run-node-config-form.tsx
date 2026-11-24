import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@ora/ui";
import {
  findVariable,
  validReferences,
  invalidComparisonField,
  isIteratorSource,
  isCollectOutput,
  supportsFeedback,
  type WorkflowAgentConfig,
  type WorkflowCapabilities,
  type WorkflowConditionCase,
  type WorkflowConditionComparison,
  type WorkflowLoopConfig,
  type WorkflowIterationConfig,
  type WorkflowVariableCatalogEntry,
} from "@ora/workflow-mock";
import { AgentExecutionFields } from "./workflow-agent-execution-fields";
import { WorkflowConditionRules } from "./workflow-condition-rules";
import { WorkflowLoopLimitFields } from "./workflow-loop-limit-fields";
import {
  IterationSelectorFields,
  IterationErrorStrategy,
  SelectorField,
  InspectorField,
} from "./workflow-node-details";

interface EditableConfig {
  agent: Pick<WorkflowAgentConfig, "executor" | "prompt">;
  condition: { cases: WorkflowConditionCase[] };
  loop: Pick<WorkflowLoopConfig, "until" | "maxIterations"> &
    Partial<Pick<WorkflowLoopConfig, "variables">>;
  iteration: Pick<
    WorkflowIterationConfig,
    "iteratorSelector" | "collectSelector"
  > &
    Partial<Pick<WorkflowIterationConfig, "errorStrategy">>;
}
export type RunNodeConfig = {
  [K in keyof EditableConfig]: { kind: K; config: EditableConfig[K] };
}[keyof EditableConfig];
type PatchPath =
  | "/agentConfig/prompt"
  | "/agentConfig/executor/modelId"
  | `/cases/${number}/logic`
  | `/cases/${number}/conditions/${number}/${"variableSelector" | "operator" | "value"}`
  | "/loopConfig/maxIterations"
  | `/loopConfig/variables/${number}/feedback`
  | "/loopConfig/until"
  | "/iterationConfig/iteratorSelector"
  | "/iterationConfig/collectSelector"
  | "/iterationConfig/errorStrategy";

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
  /** Changing the effective config discards local edits, including rich text, even at the same revision. */
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
  const [generation, setGeneration] = useState(0);
  return (
    <Draft
      key={JSON.stringify([
        props.runId,
        props.nodeRunId,
        props.baseRevision,
        props.effective,
        generation,
      ])}
      {...props}
      onCancel={() => {
        setGeneration((n) => n + 1);
        props.onCancel();
      }}
    />
  );
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
  // Preview validates the root declaration as an array, while nested traversal needs an object.
  const iteratorChoices = variables.filter(
    (entry) => isIteratorSource(entry) && entry.selector.length === 2,
  );
  const collectChoices = memberOutputs.filter(isCollectOutput);
  function invalidRuleField(rules: WorkflowConditionComparison[]) {
    for (const [offset, rule] of rules.entries()) {
      const field = invalidComparisonField(
        rule,
        variables,
        capabilities.conditionOperators,
      );
      if (field)
        return t(`settings.workflow.field.${field}`, { index: offset + 1 });
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
          showValidation
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
        const branchLabel = index === 0 ? "IF" : `ELIF ${index}`;
        if (invalid) invalidField ??= [branchLabel, invalid].join(" · ");
        return (
          <WorkflowConditionRules
            showValidation
            key={item.id}
            fixedRules={
              effective.kind === "condition"
                ? effective.config.cases[index]
                : undefined
            }
            branchLabel={branchLabel}
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
              showValidation
              {...config.until}
              variableCatalog={variables}
              operators={capabilities.conditionOperators}
              logicLabel={t("settings.workflow.loop.untilLogic")}
              onChange={(until) =>
                setDraft({ ...draft, config: { ...config, until } })
              }
            />
          </section>
          {config.variables?.map((variable, i) => {
            const label = t("settings.workflow.debug.feedback", {
              name: variable.name,
            });
            const choices = variables.filter((entry) =>
              supportsFeedback(variable.valueType, entry.valueType),
            );
            if (!findVariable(choices, variable.feedback))
              invalidField ??= label;
            return (
              <InspectorField
                key={variable.name}
                label={label}
                htmlFor={`run-loop-feedback-${i}`}
              >
                <SelectorField
                  showValidation
                  htmlId={`run-loop-feedback-${i}`}
                  selector={variable.feedback}
                  choices={choices}
                  label={label}
                  placeholder={label}
                  onPick={(feedback) =>
                    setDraft({
                      ...draft,
                      config: {
                        ...config,
                        variables: config.variables!.map((item, index) =>
                          index === i ? { ...item, feedback } : item,
                        ),
                      },
                    })
                  }
                />
              </InspectorField>
            );
          })}
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
        <>
          <IterationSelectorFields
            showValidation
            config={config}
            iteratorChoices={iteratorChoices}
            collectChoices={collectChoices}
            updateConfig={(patch) =>
              setDraft({ ...draft, config: { ...config, ...patch } })
            }
          />
          {config.errorStrategy !== undefined && (
            <IterationErrorStrategy
              value={config.errorStrategy}
              onChange={(errorStrategy) =>
                setDraft({ ...draft, config: { ...config, errorStrategy } })
              }
            />
          )}
        </>
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
      <Button type="button" variant="outline" onClick={onCancel}>
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
    draft.config.variables?.forEach((variable, i) =>
      replace(
        `/loopConfig/variables/${i}/feedback`,
        base.config.variables?.[i]?.feedback,
        variable.feedback,
      ),
    );
    replace("/loopConfig/until", base.config.until, draft.config.until);
    replace(
      "/loopConfig/maxIterations",
      base.config.maxIterations,
      draft.config.maxIterations,
    );
  } else if (base.kind === "iteration" && draft.kind === "iteration") {
    replace(
      "/iterationConfig/errorStrategy",
      base.config.errorStrategy,
      draft.config.errorStrategy,
    );
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
      entry.conditions.forEach((rule, j) => {
        const before = previous.conditions[j]!;
        for (const field of [
          "variableSelector",
          "operator",
          "value",
        ] as const) {
          if (field === "value" && before.value === undefined) continue;
          replace(
            `/cases/${i}/conditions/${j}/${field}`,
            before[field],
            rule[field] ?? null,
          );
        }
      });
    });
  }
  return patch;
}
