import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import {
  Input,
  NativeSelect,
  NativeSelectOption,
  Textarea,
  cn,
} from "@ora/ui";
import type { HitlSchema } from "@ora/workflow-runtime";

export interface WorkflowFieldFormProps {
  schema: HitlSchema;
  /** Initial values keyed by field name. */
  initialValues?: Record<string, string>;
  disabled?: boolean;
  className?: string;
  onSubmit: (values: Record<string, string>) => void | Promise<void>;
  /** Submit control rendered by the parent (receives form id). */
  formId: string;
  /** Optional external submit error (e.g. mutation failure). */
  submitError?: string | null;
}

/**
 * Shared schema field renderer for HITL (and future Kickoff schemas).
 * Validates required fields locally before calling onSubmit.
 */
export function WorkflowFieldForm({
  schema,
  initialValues = {},
  disabled = false,
  className,
  onSubmit,
  formId,
  submitError = null,
}: WorkflowFieldFormProps) {
  const { t } = useTranslation();
  const [values, setValues] = useState<Record<string, string>>(() => {
    const next: Record<string, string> = {};
    for (const field of schema.fields) {
      next[field.name] = initialValues[field.name] ?? "";
    }
    return next;
  });
  const [errors, setErrors] = useState<Record<string, string>>({});

  function setField(name: string, value: string): void {
    setValues((prev) => ({ ...prev, [name]: value }));
    setErrors((prev) => {
      if (prev[name] === undefined) {
        return prev;
      }
      const next = { ...prev };
      delete next[name];
      return next;
    });
  }

  async function handleSubmit(
    event: FormEvent<HTMLFormElement>,
  ): Promise<void> {
    event.preventDefault();
    event.stopPropagation();
    const nextErrors: Record<string, string> = {};
    for (const field of schema.fields) {
      if (field.required === true && (values[field.name] ?? "").trim() === "") {
        nextErrors[field.name] = t("workflowRun.hitl.required");
      }
    }
    setErrors(nextErrors);
    if (Object.keys(nextErrors).length > 0) {
      return;
    }
    await onSubmit(values);
  }

  return (
    <form
      id={formId}
      className={cn("space-y-3", className)}
      onSubmit={(event) => {
        void handleSubmit(event);
      }}
      onClick={(event) => event.stopPropagation()}
      onKeyDown={(event) => event.stopPropagation()}
    >
      {schema.fields.map((field) => {
        const id = `${formId}-${field.name}`;
        const error = errors[field.name];
        return (
          <div key={field.name} className="space-y-1">
            <label htmlFor={id} className="text-[11px] text-muted-foreground">
              {field.label}
              {field.required === true && (
                <span className="text-destructive" aria-hidden>
                  {" *"}
                </span>
              )}
            </label>
            {field.type === "textarea"
              ? (
                <Textarea
                  id={id}
                  value={values[field.name] ?? ""}
                  placeholder={field.placeholder}
                  disabled={disabled}
                  rows={3}
                  aria-invalid={error !== undefined}
                  className="min-h-20 resize-y text-xs leading-5"
                  onChange={(event) => setField(field.name, event.target.value)}
                />
              )
              : field.type === "select"
              ? (
                <NativeSelect
                  id={id}
                  value={values[field.name] ?? ""}
                  disabled={disabled}
                  aria-invalid={error !== undefined}
                  className="w-full"
                  onChange={(event) => setField(field.name, event.target.value)}
                >
                  <NativeSelectOption value="">
                    {t("workflowRun.hitl.selectPlaceholder")}
                  </NativeSelectOption>
                  {(field.options ?? []).map((option) => (
                    <NativeSelectOption key={option.value} value={option.value}>
                      {option.label}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
              )
              : (
                <Input
                  id={id}
                  value={values[field.name] ?? ""}
                  placeholder={field.placeholder}
                  disabled={disabled}
                  aria-invalid={error !== undefined}
                  className="h-9 text-xs"
                  onChange={(event) => setField(field.name, event.target.value)}
                />
              )}
            {error !== undefined && (
              <p role="alert" className="text-[11px] text-destructive">
                {error}
              </p>
            )}
          </div>
        );
      })}
      {submitError !== null && submitError !== "" && (
        <p role="alert" className="text-[11px] text-destructive">
          {submitError}
        </p>
      )}
    </form>
  );
}
