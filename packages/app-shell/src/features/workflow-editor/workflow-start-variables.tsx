import { useState } from "react";
import {
  IconAlignLeft,
  IconBraces,
  IconCheckbox,
  IconEdit,
  IconFile,
  IconFiles,
  IconForms,
  IconGripVertical,
  IconHash,
  IconList,
  IconPlus,
  IconTrash,
  IconVariable,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import {
  Checkbox,
  Button,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Textarea,
} from "@ora/ui";
import {
  formatWorkflowVariableValue,
  parseWorkflowVariableValueText,
  resolveWorkflowInputFieldType,
  workflowInputFieldValueType,
  workflowVariableValueExample,
  WORKFLOW_INPUT_FIELD_TYPES,
  type WorkflowInputFieldType,
  type WorkflowInputVariable,
} from "@ora/workflow-mock";

interface WorkflowStartVariablesProps {
  variables: WorkflowInputVariable[];
  onChange: (variables: WorkflowInputVariable[]) => void;
}

interface VariableDialogState {
  index: number | null;
  variable: WorkflowInputVariable;
}

/** Renders Start inputs as a compact field list and delegates edits to a focused dialog. */
export function WorkflowStartVariables({
  variables,
  onChange,
}: WorkflowStartVariablesProps) {
  const { t } = useTranslation();
  const [dialog, setDialog] = useState<VariableDialogState | null>(null);

  function openNewVariable(): void {
    setDialog({
      index: null,
      variable: {
        name: "",
        fieldType: "text-input",
        valueType: "string",
        required: false,
      },
    });
  }

  function saveVariable(variable: WorkflowInputVariable): void {
    if (dialog === null) return;
    if (dialog.index === null) {
      onChange([...variables, variable]);
    } else {
      onChange(
        variables.map((candidate, index) =>
          index === dialog.index ? variable : candidate,
        ),
      );
    }
    setDialog(null);
  }

  return (
    <>
      <section className="space-y-2">
        <div className="flex items-center justify-between gap-2">
          <h4 className="text-[11px] font-medium uppercase tracking-[0.04em] text-muted-foreground">
            {t("settings.workflow.section.inputVariables")}
          </h4>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={t("settings.workflow.start.addVariable")}
            onClick={openNewVariable}
          >
            <IconPlus className="size-4" />
          </Button>
        </div>
        {variables.length === 0 ? (
          <button
            type="button"
            className="w-full rounded-lg border border-dashed border-border px-3 py-4 text-center text-[11px] text-muted-foreground transition-colors hover:bg-muted/30"
            onClick={openNewVariable}
          >
            {t("settings.workflow.start.emptyVariables")}
          </button>
        ) : (
          <div className="space-y-1.5">
            {variables.map((variable, index) => (
              <div
                key={`${variable.name}:${index}`}
                className="group flex min-w-0 items-center gap-2 rounded-lg border border-border bg-card px-2.5 py-2 shadow-sm"
              >
                <IconGripVertical className="size-3.5 shrink-0 text-muted-foreground/60" />
                <IconVariable className="size-4 shrink-0 text-blue-600" />
                <div className="min-w-0 flex-1">
                  <div className="flex min-w-0 items-baseline gap-1">
                    <code className="truncate text-xs font-semibold text-foreground">
                      {variable.name}
                    </code>
                    {variable.displayName && (
                      <span className="truncate text-[11px] text-muted-foreground">
                        · {variable.displayName}
                      </span>
                    )}
                  </div>
                  <p className="truncate text-[10px] text-muted-foreground">
                    {t(
                      `settings.workflow.start.fieldTypes.${resolveWorkflowInputFieldType(variable)}`,
                    )}
                    {variable.maxLength !== undefined &&
                      ` · ${t("settings.workflow.start.maxLengthSummary", {
                        count: variable.maxLength,
                      })}`}
                    {" · "}
                    {variable.value === undefined
                      ? t("settings.workflow.start.configureAfterDeploy")
                      : formatWorkflowVariableValue(
                          variable.value,
                          variable.valueType,
                        )}
                  </p>
                </div>
                <StartFieldTypeIcon
                  fieldType={resolveWorkflowInputFieldType(variable)}
                  className="size-3.5 shrink-0 text-muted-foreground"
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  className="shrink-0 text-muted-foreground"
                  aria-label={t("settings.workflow.start.editVariable", {
                    name: variable.name,
                  })}
                  onClick={() => setDialog({ index, variable })}
                >
                  <IconEdit className="size-3.5" />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                  aria-label={t("settings.workflow.start.deleteVariable", {
                    name: variable.name,
                  })}
                  onClick={() =>
                    onChange(
                      variables.filter(
                        (_, candidateIndex) => candidateIndex !== index,
                      ),
                    )
                  }
                >
                  <IconTrash className="size-3.5" />
                </Button>
              </div>
            ))}
          </div>
        )}
      </section>

      {dialog !== null && (
        <WorkflowStartVariableDialog
          key={`${dialog.index ?? "new"}:${dialog.variable.name}`}
          state={dialog}
          existingNames={variables
            .filter((_, index) => index !== dialog.index)
            .map((variable) => variable.name)}
          onCancel={() => setDialog(null)}
          onSave={saveVariable}
        />
      )}
    </>
  );
}

/** Owns a temporary typed value so cancelling never mutates the graph. */
function WorkflowStartVariableDialog({
  state,
  existingNames,
  onCancel,
  onSave,
}: {
  state: VariableDialogState;
  existingNames: string[];
  onCancel: () => void;
  onSave: (variable: WorkflowInputVariable) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(state.variable.name);
  const [displayName, setDisplayName] = useState(
    state.variable.displayName ?? "",
  );
  const [fieldType, setFieldType] = useState<WorkflowInputFieldType>(() =>
    resolveWorkflowInputFieldType(state.variable),
  );
  const valueType = workflowInputFieldValueType(fieldType);
  const [required, setRequired] = useState(state.variable.required ?? false);
  const [options, setOptions] = useState<string[]>(
    state.variable.options ?? [],
  );
  const [maxLengthText, setMaxLengthText] = useState(
    state.variable.maxLength?.toString() ?? "",
  );
  const [valueText, setValueText] = useState(
    formatWorkflowVariableValue(state.variable.value, state.variable.valueType),
  );
  const [attemptedSave, setAttemptedSave] = useState(false);
  const trimmedName = name.trim();
  const parsedValue = parseWorkflowVariableValueText(valueText, valueType);
  const supportsMaxLength =
    fieldType === "text-input" || fieldType === "paragraph";
  const usesMultilineValue =
    fieldType === "paragraph" ||
    fieldType === "file-list" ||
    fieldType === "json";
  const selectOptions = options.map((option) => option.trim());
  const optionsInvalid =
    fieldType === "select" &&
    (selectOptions.length === 0 ||
      selectOptions.some((option) => option === "") ||
      new Set(selectOptions).size !== selectOptions.length);
  const selectedValueInvalid =
    fieldType === "select" &&
    parsedValue.valid &&
    parsedValue.value !== undefined &&
    !selectOptions.includes(String(parsedValue.value));
  const parsedMaxLength = Number(maxLengthText);
  const maxLengthInvalid =
    supportsMaxLength &&
    maxLengthText !== "" &&
    (!/^\d+$/.test(maxLengthText) || parsedMaxLength < 1);
  const valueExceedsMaxLength =
    supportsMaxLength &&
    !maxLengthInvalid &&
    maxLengthText !== "" &&
    parsedValue.valid &&
    typeof parsedValue.value === "string" &&
    Array.from(parsedValue.value).length > parsedMaxLength;
  const nameInvalid =
    trimmedName === "" ||
    trimmedName.includes(".") ||
    existingNames.includes(trimmedName);
  const canSave =
    !nameInvalid &&
    parsedValue.valid &&
    !maxLengthInvalid &&
    !valueExceedsMaxLength &&
    !optionsInvalid &&
    !selectedValueInvalid;

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {state.index === null
              ? t("settings.workflow.start.createVariableTitle")
              : t("settings.workflow.start.editVariableTitle")}
          </DialogTitle>
        </DialogHeader>
        <div className="space-y-5 py-2">
          <div className="space-y-1.5">
            <Label htmlFor="workflow-start-variable-type">
              {t("settings.workflow.start.fieldType")}
            </Label>
            <Select
              value={fieldType}
              onValueChange={(candidate) => {
                if (candidate === null || !isWorkflowInputFieldType(candidate))
                  return;
                setFieldType(candidate);
                setValueText("");
                if (candidate !== "select") setOptions([]);
              }}
            >
              <SelectTrigger
                id="workflow-start-variable-type"
                className="w-full bg-muted/45"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {WORKFLOW_INPUT_FIELD_TYPES.map((candidate) => (
                  <SelectItem key={candidate} value={candidate}>
                    <span className="flex w-full items-center justify-between gap-4">
                      <span className="flex items-center gap-2">
                        <StartFieldTypeIcon
                          fieldType={candidate}
                          className="size-4 text-muted-foreground"
                        />
                        {t(`settings.workflow.start.fieldTypes.${candidate}`)}
                      </span>
                      <code className="text-[10px] text-muted-foreground">
                        {workflowInputFieldValueType(candidate)}
                      </code>
                    </span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="workflow-start-variable-name">
              {t("settings.workflow.start.variableName")}
            </Label>
            <Input
              id="workflow-start-variable-name"
              className="bg-muted/45"
              value={name}
              aria-invalid={attemptedSave && nameInvalid}
              placeholder={t(
                "settings.workflow.field.inputVariableNamePlaceholder",
              )}
              onChange={(event) => setName(event.target.value)}
            />
            {attemptedSave && nameInvalid && (
              <p className="text-[11px] text-destructive" role="status">
                {t("settings.workflow.start.variableNameInvalid")}
              </p>
            )}
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="workflow-start-variable-display-name">
              {t("settings.workflow.start.displayName")}
              <span className="ml-1 font-normal text-muted-foreground">
                {t("settings.workflow.start.optional")}
              </span>
            </Label>
            <Input
              id="workflow-start-variable-display-name"
              className="bg-muted/45"
              value={displayName}
              placeholder={t("settings.workflow.start.displayNamePlaceholder")}
              onChange={(event) => setDisplayName(event.target.value)}
            />
          </div>
          {supportsMaxLength && (
            <div className="space-y-1.5">
              <Label htmlFor="workflow-start-variable-max-length">
                {t("settings.workflow.start.maxLength")}
                <span className="ml-1 font-normal text-muted-foreground">
                  {t("settings.workflow.start.optional")}
                </span>
              </Label>
              <Input
                id="workflow-start-variable-max-length"
                className="bg-muted/45"
                type="number"
                min={1}
                step={1}
                value={maxLengthText}
                aria-invalid={attemptedSave && maxLengthInvalid}
                placeholder={t("settings.workflow.start.maxLengthPlaceholder")}
                onChange={(event) => setMaxLengthText(event.target.value)}
              />
              {attemptedSave && maxLengthInvalid && (
                <p className="text-[11px] text-destructive" role="status">
                  {t("settings.workflow.start.maxLengthInvalid")}
                </p>
              )}
            </div>
          )}
          {fieldType === "select" && (
            <div className="space-y-2">
              <Label>{t("settings.workflow.start.options")}</Label>
              {options.map((option, index) => (
                <div key={index} className="flex items-center gap-2">
                  <Input
                    className="bg-muted/45"
                    value={option}
                    aria-label={t("settings.workflow.start.option", {
                      index: index + 1,
                    })}
                    onChange={(event) =>
                      setOptions((current) =>
                        current.map((candidate, candidateIndex) =>
                          candidateIndex === index
                            ? event.target.value
                            : candidate,
                        ),
                      )
                    }
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    aria-label={t("settings.workflow.start.deleteOption", {
                      index: index + 1,
                    })}
                    onClick={() =>
                      setOptions((current) =>
                        current.filter(
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
                className="w-full"
                onClick={() => setOptions((current) => [...current, ""])}
              >
                <IconPlus className="size-3.5" />
                {t("settings.workflow.start.addOption")}
              </Button>
              {attemptedSave && optionsInvalid && (
                <p className="text-[11px] text-destructive" role="status">
                  {t("settings.workflow.start.optionsInvalid")}
                </p>
              )}
            </div>
          )}
          <div className="space-y-1.5">
            <Label htmlFor="workflow-start-variable-value">
              {t("settings.workflow.start.initialValue")}
              <span className="ml-1 font-normal text-muted-foreground">
                {t("settings.workflow.start.optional")}
              </span>
            </Label>
            {fieldType === "checkbox" ? (
              <label className="flex h-10 items-center gap-2 rounded-md bg-muted/45 px-3 text-sm">
                <Checkbox
                  checked={valueText === "true"}
                  onCheckedChange={(checked) =>
                    setValueText(checked === true ? "true" : "false")
                  }
                />
                {valueText === "true"
                  ? t("settings.workflow.start.checked")
                  : t("settings.workflow.start.unchecked")}
              </label>
            ) : usesMultilineValue ? (
              <Textarea
                id="workflow-start-variable-value"
                className="min-h-20 bg-muted/45"
                value={valueText}
                aria-invalid={
                  attemptedSave && (!parsedValue.valid || valueExceedsMaxLength)
                }
                placeholder={workflowVariableValueExample(valueType)}
                onChange={(event) => setValueText(event.target.value)}
              />
            ) : fieldType === "select" ? (
              <Select
                value={valueText || null}
                onValueChange={(value) => setValueText(value ?? "")}
              >
                <SelectTrigger
                  id="workflow-start-variable-value"
                  className="w-full bg-muted/45"
                >
                  <SelectValue
                    placeholder={t("settings.workflow.start.noInitialValue")}
                  />
                </SelectTrigger>
                <SelectContent>
                  {selectOptions.filter(Boolean).map((option) => (
                    <SelectItem key={option} value={option}>
                      {option}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : (
              <Input
                id="workflow-start-variable-value"
                className="bg-muted/45"
                type={fieldType === "number" ? "number" : "text"}
                value={valueText}
                aria-invalid={
                  attemptedSave && (!parsedValue.valid || valueExceedsMaxLength)
                }
                placeholder={workflowVariableValueExample(valueType)}
                onChange={(event) => setValueText(event.target.value)}
              />
            )}
            {attemptedSave && !parsedValue.valid && (
              <p className="text-[11px] text-destructive" role="status">
                {t("settings.workflow.variableValueInvalid", {
                  type: valueType,
                })}
              </p>
            )}
            {attemptedSave && selectedValueInvalid && (
              <p className="text-[11px] text-destructive" role="status">
                {t("settings.workflow.start.initialOptionInvalid")}
              </p>
            )}
            {attemptedSave && valueExceedsMaxLength && (
              <p className="text-[11px] text-destructive" role="status">
                {t("settings.workflow.start.valueTooLong", {
                  count: parsedMaxLength,
                })}
              </p>
            )}
            <p className="text-[10px] leading-4 text-muted-foreground">
              {t("settings.workflow.start.initialValueHint")}
            </p>
          </div>
          <label className="flex items-center gap-2 text-sm font-medium">
            <Checkbox
              checked={required}
              onCheckedChange={(checked) => setRequired(checked === true)}
            />
            {t("settings.workflow.start.required")}
          </label>
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={onCancel}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            onClick={() => {
              setAttemptedSave(true);
              if (!canSave || !parsedValue.valid) return;
              onSave({
                name: trimmedName,
                ...(displayName.trim() === ""
                  ? {}
                  : { displayName: displayName.trim() }),
                valueType,
                fieldType,
                required,
                ...(fieldType === "select" ? { options: selectOptions } : {}),
                ...(supportsMaxLength && maxLengthText !== ""
                  ? { maxLength: parsedMaxLength }
                  : {}),
                ...(parsedValue.value === undefined
                  ? {}
                  : { value: parsedValue.value }),
              });
            }}
          >
            {t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Narrows the select's string value to a supported Start form control. */
function isWorkflowInputFieldType(
  value: string,
): value is WorkflowInputFieldType {
  return WORKFLOW_INPUT_FIELD_TYPES.includes(value as WorkflowInputFieldType);
}

/** Gives each Start form control a stable visual identity in lists and selectors. */
function StartFieldTypeIcon({
  fieldType,
  className,
}: {
  fieldType: WorkflowInputFieldType;
  className?: string;
}) {
  switch (fieldType) {
    case "text-input":
      return <IconForms className={className} />;
    case "paragraph":
      return <IconAlignLeft className={className} />;
    case "select":
      return <IconList className={className} />;
    case "number":
      return <IconHash className={className} />;
    case "checkbox":
      return <IconCheckbox className={className} />;
    case "file":
      return <IconFile className={className} />;
    case "file-list":
      return <IconFiles className={className} />;
    case "json":
      return <IconBraces className={className} />;
  }
}
