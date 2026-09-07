import { useState } from "react";
import { IconPlus, IconTrash, IconVariable } from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@ora/ui";
import {
  formatWorkflowVariableValue,
  normalizeWorkflowVariableValue,
  normalizeWorkflowGlobalVariables,
  parseWorkflowVariableValueText,
  WORKFLOW_VARIABLE_VALUE_TYPES,
  workflowVariableValueExample,
  type WorkflowGlobalVariable,
  type WorkflowVariableValueType,
} from "@ora/workflow-mock";

const SYSTEM_GLOBALS = new Set(["sys.workflow_id", "sys.timestamp"]);
interface WorkflowGlobalVariablesDialogProps {
  open: boolean;
  variables: WorkflowGlobalVariable[];
  onOpenChange: (open: boolean) => void;
  onSave: (variables: WorkflowGlobalVariable[]) => void;
}

/** Edits workflow-wide variables while protecting runtime-owned system declarations. */
export function WorkflowGlobalVariablesDialog({
  open,
  variables,
  onOpenChange,
  onSave,
}: WorkflowGlobalVariablesDialogProps) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(() =>
    normalizeWorkflowGlobalVariables(variables),
  );
  const hasInvalidCustomVariables = draft.some(
    (variable) =>
      !isSystemGlobal(variable) &&
      (!variable.name.includes(".") ||
        variable.value === undefined ||
        !normalizeWorkflowVariableValue(variable.value, variable.valueType)
          .valid),
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t("settings.workflow.globalVariables")}</DialogTitle>
          <DialogDescription>
            {t("settings.workflow.globalVariablesDescription")}
          </DialogDescription>
        </DialogHeader>
        <div className="max-h-[60vh] space-y-5 overflow-y-auto pr-1">
          <section className="space-y-2">
            <div>
              <h3 className="text-sm font-semibold">
                {t("settings.workflow.systemVariables")}
              </h3>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("settings.workflow.systemVariablesDescription")}
              </p>
            </div>
            {draft.filter(isSystemGlobal).map((variable) => (
              <div
                key={variable.name}
                className="rounded-lg border border-border bg-card px-3 py-2 shadow-sm"
              >
                <div className="flex items-center gap-1.5 text-sm">
                  <IconVariable className="size-4 text-orange-600" />
                  <code className="font-semibold text-foreground">
                    {variable.name}
                  </code>
                  <span className="text-xs font-medium capitalize text-muted-foreground">
                    {variable.valueType}
                  </span>
                </div>
                <p className="mt-1 text-xs text-muted-foreground">
                  {t(
                    variable.name === "sys.workflow_id"
                      ? "settings.workflow.workflowIdDescription"
                      : "settings.workflow.timestampDescription",
                  )}
                </p>
              </div>
            ))}
          </section>

          <section className="space-y-2">
            <div>
              <h3 className="text-sm font-semibold">
                {t("settings.workflow.customVariables")}
              </h3>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("settings.workflow.customVariablesDescription")}
              </p>
            </div>
            {draft.map((variable, index) => {
              if (SYSTEM_GLOBALS.has(variable.name)) {
                return null;
              }
              return (
                <div
                  // The name is editable, so the row position stays stable while the user types.
                  key={index}
                  className="space-y-1 rounded-lg border border-border p-2"
                >
                  <div className="grid grid-cols-[minmax(0,1.5fr)_minmax(130px,0.75fr)_minmax(0,1.5fr)_32px] items-center gap-2">
                    <Input
                      value={variable.name}
                      required
                      aria-invalid={!variable.name.includes(".")}
                      aria-label={t("settings.workflow.globalVariableName", {
                        index: index + 1,
                      })}
                      placeholder="global.variable_name"
                      onChange={(event) =>
                        updateVariable(index, { name: event.target.value })
                      }
                    />
                    <Select
                      value={variable.valueType}
                      onValueChange={(valueType) => {
                        if (isValueType(valueType)) {
                          updateVariable(index, {
                            valueType,
                            value: undefined,
                          });
                        }
                      }}
                    >
                      <SelectTrigger
                        aria-label={t("settings.workflow.globalVariableType", {
                          index: index + 1,
                        })}
                      >
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {WORKFLOW_VARIABLE_VALUE_TYPES.map((valueType) => (
                          <SelectItem key={valueType} value={valueType}>
                            {valueType}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Input
                      value={formatWorkflowVariableValue(
                        variable.value,
                        variable.valueType,
                      )}
                      required
                      aria-invalid={
                        variable.value === undefined ||
                        !normalizeWorkflowVariableValue(
                          variable.value,
                          variable.valueType,
                        ).valid
                      }
                      type={
                        variable.valueType === "secret" ? "password" : "text"
                      }
                      aria-label={t("settings.workflow.globalVariableValue", {
                        index: index + 1,
                      })}
                      placeholder={t("settings.workflow.valueExample", {
                        example: workflowVariableValueExample(
                          variable.valueType,
                        ),
                      })}
                      onChange={(event) => {
                        const result = parseWorkflowVariableValueText(
                          event.target.value,
                          variable.valueType,
                        );
                        updateVariable(index, {
                          value: result.valid
                            ? result.value
                            : event.target.value,
                        });
                      }}
                    />
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t("settings.workflow.removeGlobalVariable", {
                        index: index + 1,
                      })}
                      onClick={() =>
                        setDraft((current) =>
                          current.filter((_, candidate) => candidate !== index),
                        )
                      }
                    >
                      <IconTrash />
                    </Button>
                  </div>
                  {variable.value !== undefined &&
                    !normalizeWorkflowVariableValue(
                      variable.value,
                      variable.valueType,
                    ).valid && (
                      <p className="px-1 text-[11px] text-destructive">
                        {t("settings.workflow.variableValueInvalid", {
                          type: variable.valueType,
                        })}
                      </p>
                    )}
                </div>
              );
            })}
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="w-full"
              onClick={() =>
                setDraft((current) => [
                  ...current,
                  {
                    name: "",
                    valueType: "string",
                  },
                ])
              }
            >
              <IconPlus />
              {t("settings.workflow.addGlobalVariable")}
            </Button>
          </section>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            disabled={hasInvalidCustomVariables}
            onClick={() => {
              onSave(
                normalizeWorkflowGlobalVariables(
                  draft.filter((variable) => variable.name.includes(".")),
                ),
              );
              onOpenChange(false);
            }}
          >
            {t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );

  /** Applies one field edit without changing other variable declarations. */
  function updateVariable(
    index: number,
    patch: Partial<WorkflowGlobalVariable>,
  ): void {
    setDraft((current) =>
      current.map((variable, candidate) =>
        candidate === index ? { ...variable, ...patch } : variable,
      ),
    );
  }
}

/** Identifies declarations whose values and types belong to the workflow runtime. */
function isSystemGlobal(variable: WorkflowGlobalVariable): boolean {
  return SYSTEM_GLOBALS.has(variable.name);
}

/** Narrows a Select value to the supported variable vocabulary. */
function isValueType(value: string | null): value is WorkflowVariableValueType {
  return (
    value !== null &&
    WORKFLOW_VARIABLE_VALUE_TYPES.includes(value as WorkflowVariableValueType)
  );
}
