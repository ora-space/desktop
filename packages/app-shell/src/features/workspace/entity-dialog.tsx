import { useMemo, useRef, useState, type FormEvent } from "react";
import { IconFolderOpen } from "@tabler/icons-react";
import { usePlatform, type PathSelectionKind } from "../../platform";
import {
  Button,
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
} from "@ora/ui";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";

interface EntityFieldBase {
  name: string;
  label: string;
  value: string;
}

interface TextEntityField extends EntityFieldBase {
  kind: "text";
  placeholder?: string;
}

interface SelectEntityField extends EntityFieldBase {
  kind: "select";
  options: Array<{ label: string; value: string }>;
  /** True while options are still loading, so submit cannot race an empty value. */
  loading?: boolean;
}

interface PathEntityField extends EntityFieldBase {
  kind: "path";
  selectionKind: PathSelectionKind;
  placeholder?: string;
}

export type EntityField = TextEntityField | SelectEntityField | PathEntityField;

interface EntityDialogProps {
  open: boolean;
  title: string;
  description?: string;
  submitLabel: string;
  /** In-flight label; create flows pass a creating verb instead of "Saving…". */
  pendingLabel?: string;
  fields: EntityField[];
  onOpenChange: (open: boolean) => void;
  onSubmit: (values: Record<string, string>) => Promise<void>;
}

/** Provides one consistent create/edit form for every level of the workspace tree. */
export function EntityDialog({
  open,
  title,
  description,
  submitLabel,
  pendingLabel,
  fields,
  onOpenChange,
  onSubmit,
}: EntityDialogProps) {
  const { t } = useTranslation();
  const platform = usePlatform();
  // Lazy-init from fields; callers pass a `key` to remount when the entity changes.
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(fields.map((field) => [field.name, field.value])),
  );
  const [submitting, setSubmitting] = useState(false);
  // Ref closes the window between the first submit and the disabled re-render.
  const submittingRef = useRef(false);
  const [validationError, setValidationError] = useState(false);
  const [optionsLoadingError, setOptionsLoadingError] = useState(false);
  const [selectingField, setSelectingField] = useState<string | null>(null);
  const [pathSelectionError, setPathSelectionError] = useState<string | null>(
    null,
  );
  const [submissionError, setSubmissionError] = useState<string | null>(null);
  const inFlightLabel = pendingLabel ?? t("common.saving");

  const resolvedValues = useMemo(() => {
    // Select options arrive asynchronously for repository-backed forms. Fill only
    // untouched values so a late query cannot overwrite input the user changed.
    let next = values;
    for (const field of fields) {
      if ((values[field.name] ?? "") === "" && field.value !== "") {
        next = { ...next, [field.name]: field.value };
      }
    }
    return next;
  }, [fields, values]);

  const optionsLoading = fields.some(
    (field) => field.kind === "select" && field.loading === true,
  );
  // A loading select is empty because options have not arrived, not because the
  // user skipped it. Treat that as a wait state so Enter does not look like a
  // required-field miss that then lingers after the default branch fills in.
  const hasEmptyNonLoadingField = fields.some(
    (field) =>
      !(field.kind === "select" && field.loading === true) &&
      !resolvedValues[field.name]?.trim(),
  );

  const handlePathSelection = async (field: PathEntityField) => {
    setSelectingField(field.name);
    setPathSelectionError(null);
    try {
      const initialPath = resolvedValues[field.name]?.trim();
      const selectedPath = await platform.selectPath({
        kind: field.selectionKind,
        initialPath: initialPath === "" ? undefined : initialPath,
      });
      if (selectedPath !== null) {
        setValues((current) => ({ ...current, [field.name]: selectedPath }));
        setValidationError(false);
      }
    } catch {
      setPathSelectionError(field.name);
    } finally {
      setSelectingField(null);
    }
  };

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (submittingRef.current) return;
    // Native Enter still submits a form whose submit button is disabled, so
    // keep the same validation/feedback path the click path would have shown.
    if (hasEmptyNonLoadingField) {
      setValidationError(true);
      setOptionsLoadingError(false);
      return;
    }
    if (optionsLoading) {
      setOptionsLoadingError(true);
      setValidationError(false);
      return;
    }
    submittingRef.current = true;
    setSubmitting(true);
    setSubmissionError(null);
    setOptionsLoadingError(false);
    try {
      await onSubmit(resolvedValues);
      onOpenChange(false);
    } catch (error) {
      setSubmissionError(localizeContractError(error, t));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) =>
        (!submitting || nextOpen) && onOpenChange(nextOpen)
      }
    >
      <DialogContent>
        <form onSubmit={handleSubmit} className="contents">
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            {description && (
              <DialogDescription>{description}</DialogDescription>
            )}
          </DialogHeader>
          <div className="grid gap-3">
            {fields.map((field) => (
              <div key={field.name} className="grid gap-1.5">
                <Label htmlFor={`entity-${field.name}`}>{field.label}</Label>
                {field.kind === "select" ? (
                  <Select
                    value={resolvedValues[field.name] ?? ""}
                    disabled={submitting || field.loading === true}
                    onValueChange={(value) =>
                      setValues((current) => ({
                        ...current,
                        [field.name]: value ?? "",
                      }))
                    }
                  >
                    <SelectTrigger
                      id={`entity-${field.name}`}
                      className="w-full"
                      aria-busy={field.loading === true}
                    >
                      {field.loading === true ? (
                        <Spinner
                          className="size-3.5 text-muted-foreground"
                          aria-hidden="true"
                        />
                      ) : (
                        <SelectValue />
                      )}
                    </SelectTrigger>
                    <SelectContent>
                      {field.options.map((option) => (
                        <SelectItem key={option.value} value={option.value}>
                          {option.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                ) : field.kind === "path" ? (
                  <div className="flex gap-2">
                    <Input
                      id={`entity-${field.name}`}
                      className="min-w-0 flex-1"
                      value={values[field.name] ?? ""}
                      placeholder={field.placeholder}
                      disabled={submitting}
                      aria-invalid={
                        validationError && !resolvedValues[field.name]?.trim()
                      }
                      onChange={(event) => {
                        setValues((current) => ({
                          ...current,
                          [field.name]: event.target.value,
                        }));
                        setValidationError(false);
                        setPathSelectionError(null);
                      }}
                      autoFocus={field === fields[0]}
                    />
                    <Button
                      type="button"
                      variant="outline"
                      disabled={submitting || selectingField !== null}
                      onClick={() => void handlePathSelection(field)}
                    >
                      <IconFolderOpen />
                      {t("common.browse")}
                    </Button>
                  </div>
                ) : (
                  <Input
                    id={`entity-${field.name}`}
                    value={resolvedValues[field.name] ?? ""}
                    placeholder={field.placeholder}
                    disabled={submitting}
                    aria-invalid={
                      validationError && !resolvedValues[field.name]?.trim()
                    }
                    onChange={(event) => {
                      setValues((current) => ({
                        ...current,
                        [field.name]: event.target.value,
                      }));
                      setValidationError(false);
                    }}
                    autoFocus={field === fields[0]}
                  />
                )}
                {pathSelectionError === field.name && (
                  <p
                    role="alert"
                    data-selectable
                    className="text-xs text-destructive"
                  >
                    {t("dialog.pathSelectionError")}
                  </p>
                )}
              </div>
            ))}
          </div>
          {validationError && hasEmptyNonLoadingField && (
            <p role="alert" className="text-xs text-destructive">
              {t("dialog.required")}
            </p>
          )}
          {optionsLoadingError && optionsLoading && (
            <p role="alert" className="text-xs text-destructive">
              {t("dialog.optionsLoading")}
            </p>
          )}
          {submissionError && (
            <p
              role="alert"
              data-selectable
              className="text-xs text-destructive"
            >
              {submissionError}
            </p>
          )}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={submitting}
              onClick={() => onOpenChange(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              disabled={submitting || optionsLoading}
              aria-busy={submitting || optionsLoading}
            >
              {submitting || optionsLoading ? (
                <Spinner className="size-3.5" aria-hidden="true" />
              ) : null}
              {submitting ? inFlightLabel : submitLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
