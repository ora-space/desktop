import { useState, type Dispatch, type SetStateAction } from "react";
import { useTranslation } from "react-i18next";
import {
  Button,
  Checkbox,
  Dialog,
  DialogContent,
  DialogDescription,
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
  Spinner,
  Textarea,
} from "@ora/ui";
import {
  formatWorkflowVariableValue,
  parseWorkflowVariableValueText,
  resolveWorkflowInputFieldType,
  workflowVariableValueExample,
  type WorkflowInputVariable,
} from "@ora/workflow-mock";

interface WorkflowRunStartDialogProps {
  open: boolean;
  runName: string;
  /** The free-text run instruction, edited here as the workflow's "initial prompt". */
  initialPrompt: string;
  variables: WorkflowInputVariable[];
  busy: boolean;
  onOpenChange: (open: boolean) => void;
  onStart: (
    initialPrompt: string,
    variables: Record<string, unknown>,
  ) => Promise<void>;
}

/** Collects the run instruction and deployment-time Start values before execution. */
export function WorkflowRunStartDialog({
  open,
  runName,
  initialPrompt,
  variables,
  busy,
  onOpenChange,
  onStart,
}: WorkflowRunStartDialogProps) {
  return (
    <Dialog open={open} onOpenChange={(next) => !busy && onOpenChange(next)}>
      {open && (
        <WorkflowRunStartDialogContent
          runName={runName}
          initialPrompt={initialPrompt}
          variables={variables}
          busy={busy}
          onCancel={() => onOpenChange(false)}
          onStart={onStart}
        />
      )}
    </Dialog>
  );
}

/** Keeps form drafts local so closing the dialog leaves persisted run values untouched. */
function WorkflowRunStartDialogContent({
  runName,
  initialPrompt,
  variables,
  busy,
  onCancel,
  onStart,
}: Omit<WorkflowRunStartDialogProps, "open" | "onOpenChange"> & {
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const [initialPromptDraft, setInitialPromptDraft] = useState(initialPrompt);
  const [drafts, setDrafts] = useState<Record<string, string>>(() =>
    Object.fromEntries(
      variables.map((variable) => [
        variable.name,
        resolveWorkflowInputFieldType(variable) === "checkbox" &&
        variable.value === undefined
          ? "false"
          : formatWorkflowVariableValue(variable.value, variable.valueType),
      ]),
    ),
  );
  const [attemptedStart, setAttemptedStart] = useState(false);
  const parsed = variables.map((variable) => {
    const result = parseWorkflowVariableValueText(
      drafts[variable.name] ?? "",
      variable.valueType,
    );
    return {
      variable,
      result,
      missingRequired:
        variable.required === true &&
        result.valid &&
        result.value === undefined,
      exceedsMaxLength:
        variable.maxLength !== undefined &&
        result.valid &&
        typeof result.value === "string" &&
        Array.from(result.value).length > variable.maxLength,
    };
  });
  const hasInvalidValue = parsed.some(
    ({ result, exceedsMaxLength, missingRequired }) =>
      !result.valid || exceedsMaxLength || missingRequired,
  );

  async function submit(): Promise<void> {
    setAttemptedStart(true);
    if (hasInvalidValue) return;
    const values = Object.fromEntries(
      parsed.map(({ variable, result }) => [
        variable.name,
        result.valid && result.value !== undefined ? result.value : null,
      ]),
    );
    await onStart(initialPromptDraft, values);
  }

  return (
    <DialogContent className="gap-0 overflow-hidden p-0 sm:max-w-lg">
      <DialogHeader className="border-b border-border px-5 py-4">
        <DialogTitle>{runName}</DialogTitle>
        <DialogDescription>
          {t("workflowRun.startInputsDescription")}
        </DialogDescription>
      </DialogHeader>
      <div className="max-h-[min(62vh,560px)] space-y-4 overflow-y-auto px-5 py-5">
        {variables.length === 0 ? (
          <p className="rounded-lg bg-muted/45 px-3 py-5 text-center text-xs text-muted-foreground">
            {t("workflowRun.startInputsEmpty")}
          </p>
        ) : (
          variables.map((variable, index) => {
            const parsedVariable = parsed[index];
            const result = parsedVariable?.result;
            const fieldType = resolveWorkflowInputFieldType(variable);
            const inputId = `workflow-run-start-${variable.name}`;
            const value = drafts[variable.name] ?? "";
            const invalid =
              attemptedStart &&
              (result?.valid === false ||
                parsedVariable?.missingRequired === true ||
                parsedVariable?.exceedsMaxLength === true);
            return (
              <div key={variable.name} className="space-y-1.5">
                <div className="flex items-baseline justify-between gap-2">
                  <Label htmlFor={inputId}>
                    {variable.displayName?.trim() || variable.name}
                  </Label>
                  <span className="text-[10px] text-muted-foreground">
                    {variable.required === true
                      ? t("workflowRun.inputRequired")
                      : t("workflowRun.inputOptional")}
                  </span>
                </div>
                {fieldType === "paragraph" ||
                fieldType === "file-list" ||
                fieldType === "json" ? (
                  <Textarea
                    id={inputId}
                    className="min-h-24 bg-muted/55"
                    value={value}
                    aria-invalid={invalid}
                    maxLength={variable.maxLength}
                    placeholder={workflowVariableValueExample(
                      variable.valueType,
                    )}
                    onChange={(event) =>
                      updateDraft(setDrafts, variable.name, event.target.value)
                    }
                  />
                ) : fieldType === "select" ? (
                  <Select
                    value={value || null}
                    onValueChange={(next) =>
                      updateDraft(setDrafts, variable.name, next ?? "")
                    }
                  >
                    <SelectTrigger
                      id={inputId}
                      className="w-full bg-muted/55"
                      aria-invalid={invalid}
                    >
                      <SelectValue
                        placeholder={t("workflowRun.selectPlaceholder")}
                      />
                    </SelectTrigger>
                    <SelectContent>
                      {(variable.options ?? []).map((option) => (
                        <SelectItem key={option} value={option}>
                          {option}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                ) : fieldType === "checkbox" ? (
                  <label
                    htmlFor={inputId}
                    className="flex h-10 items-center gap-2 rounded-md bg-muted/55 px-3 text-sm"
                  >
                    <Checkbox
                      id={inputId}
                      checked={value === "true"}
                      aria-invalid={invalid}
                      onCheckedChange={(checked) =>
                        updateDraft(
                          setDrafts,
                          variable.name,
                          checked === true ? "true" : "false",
                        )
                      }
                    />
                    {value === "true"
                      ? t("settings.workflow.start.checked")
                      : t("settings.workflow.start.unchecked")}
                  </label>
                ) : (
                  <Input
                    id={inputId}
                    className="h-10 bg-muted/55"
                    type={fieldType === "number" ? "number" : "text"}
                    value={value}
                    aria-invalid={invalid}
                    maxLength={variable.maxLength}
                    placeholder={workflowVariableValueExample(
                      variable.valueType,
                    )}
                    onChange={(event) =>
                      updateDraft(setDrafts, variable.name, event.target.value)
                    }
                  />
                )}
                {attemptedStart && result?.valid === false && (
                  <p className="text-[11px] text-destructive" role="status">
                    {t("settings.workflow.variableValueInvalid", {
                      type: variable.valueType,
                    })}
                  </p>
                )}
                {attemptedStart && parsedVariable?.exceedsMaxLength && (
                  <p className="text-[11px] text-destructive" role="status">
                    {t("settings.workflow.start.valueTooLong", {
                      count: variable.maxLength,
                    })}
                  </p>
                )}
                {attemptedStart && parsedVariable?.missingRequired && (
                  <p className="text-[11px] text-destructive" role="status">
                    {t("workflowRun.inputRequiredError")}
                  </p>
                )}
              </div>
            );
          })
        )}
        <div className="space-y-1.5">
          <div className="flex items-baseline justify-between gap-2">
            <Label htmlFor="workflow-run-start-prompt">
              {t("settings.workflow.field.initialPrompt")}
            </Label>
            <span className="text-[10px] text-muted-foreground">
              {t("workflowRun.inputOptional")}
            </span>
          </div>
          <Textarea
            id="workflow-run-start-prompt"
            className="min-h-24 bg-muted/55"
            value={initialPromptDraft}
            onChange={(event) => setInitialPromptDraft(event.target.value)}
          />
        </div>
      </div>
      <DialogFooter className="mb-0 border-t border-border px-5 py-4">
        <Button
          type="button"
          variant="outline"
          className="min-w-24"
          disabled={busy}
          onClick={onCancel}
        >
          {t("common.cancel")}
        </Button>
        <Button
          type="button"
          className="min-w-28"
          disabled={busy}
          onClick={() => void submit()}
        >
          {busy ? (
            <span className="inline-flex items-center gap-1.5">
              <Spinner className="size-3.5" />
              {t("workflowRun.starting")}
            </span>
          ) : (
            t("workflowRun.startConfirm")
          )}
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}

/** Replaces one field draft without coupling the individual controls to form state shape. */
function updateDraft(
  setDrafts: Dispatch<SetStateAction<Record<string, string>>>,
  name: string,
  value: string,
): void {
  setDrafts((current) => ({ ...current, [name]: value }));
}
