import { useState, type FormEvent } from "react";
import { IconFolderOpen } from "@tabler/icons-react";
import { usePlatform, type PathSelectionKind } from "@ora/platform";
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
} from "@ora/ui";
import { useTranslation } from "react-i18next";

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
  description: string;
  submitLabel: string;
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
  const [validationError, setValidationError] = useState(false);
  const [selectingField, setSelectingField] = useState<string | null>(null);
  const [pathSelectionError, setPathSelectionError] = useState<string | null>(null);

  const handlePathSelection = async (field: PathEntityField) => {
    setSelectingField(field.name);
    setPathSelectionError(null);
    try {
      const initialPath = values[field.name]?.trim();
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
    if (fields.some((field) => !values[field.name]?.trim())) {
      setValidationError(true);
      return;
    }
    setSubmitting(true);
    try {
      await onSubmit(values);
      onOpenChange(false);
    } catch {
      // The workspace surfaces transport errors inline, so the form stays open for correction or retry.
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => (!submitting || nextOpen) && onOpenChange(nextOpen)}>
      <DialogContent>
        <form onSubmit={handleSubmit} className="contents">
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-3">
            {fields.map((field) => (
              <div key={field.name} className="grid gap-1.5">
                <Label htmlFor={`entity-${field.name}`}>{field.label}</Label>
                {field.kind === "select" ? (
                  <Select
                    value={values[field.name] ?? ""}
                    onValueChange={(value) => setValues((current) => ({ ...current, [field.name]: value ?? "" }))}
                  >
                    <SelectTrigger id={`entity-${field.name}`} className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {field.options.map((option) => (
                        <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>
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
                      aria-invalid={validationError && !values[field.name]?.trim()}
                      onChange={(event) => {
                        setValues((current) => ({ ...current, [field.name]: event.target.value }));
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
                    value={values[field.name] ?? ""}
                    placeholder={field.placeholder}
                    aria-invalid={validationError && !values[field.name]?.trim()}
                    onChange={(event) => {
                      setValues((current) => ({ ...current, [field.name]: event.target.value }));
                      setValidationError(false);
                    }}
                    autoFocus={field === fields[0]}
                  />
                )}
                {pathSelectionError === field.name && (
                  <p role="alert" className="text-xs text-destructive">
                    {t("dialog.pathSelectionError")}
                  </p>
                )}
              </div>
            ))}
          </div>
          {validationError && <p role="alert" className="text-xs text-destructive">{t("dialog.required")}</p>}
          <DialogFooter>
            <Button type="button" variant="outline" disabled={submitting} onClick={() => onOpenChange(false)}>{t("common.cancel")}</Button>
            <Button type="submit" disabled={submitting}>{submitting ? t("common.saving") : submitLabel}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
