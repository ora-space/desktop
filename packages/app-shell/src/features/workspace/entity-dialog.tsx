import { useMemo, useRef, useState, type FormEvent } from "react";
import { IconFolderOpen, IconSearch } from "@tabler/icons-react";
import { usePlatform, type PathSelectionKind } from "../../platform";
import {
  Button,
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Popover,
  PopoverContent,
  PopoverTrigger,
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

interface SelectOption {
  label: string;
  value: string;
  /** Extra text that search matches, such as the git branch behind a task title. */
  keywords?: string;
}

interface SelectEntityField extends EntityFieldBase {
  kind: "select";
  options: Array<{ label: string; value: string }>;
  /** True while options are still loading, so submit cannot race an empty value. */
  loading?: boolean;
}

interface SearchableSelectEntityField extends EntityFieldBase {
  kind: "searchable-select";
  options: SelectOption[];
  /** True while options are still loading, so submit cannot race an empty value. */
  loading?: boolean;
}

interface PathEntityField extends EntityFieldBase {
  kind: "path";
  selectionKind: PathSelectionKind;
  placeholder?: string;
}

export type EntityField =
  | TextEntityField
  | SelectEntityField
  | SearchableSelectEntityField
  | PathEntityField;

function isChoiceField(
  field: EntityField,
): field is SelectEntityField | SearchableSelectEntityField {
  return field.kind === "select" || field.kind === "searchable-select";
}

/** Shows every option while the query is empty or still the current label, then matches label, keywords, and value. */
function optionMatchesQuery(
  option: SelectOption,
  query: string,
  selectedLabel: string,
): boolean {
  const needle = query.trim().toLowerCase();
  if (needle === "" || needle === selectedLabel.trim().toLowerCase())
    return true;
  return [option.label, option.keywords, option.value].some((part) =>
    part?.toLowerCase().includes(needle),
  );
}

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
    (field) => isChoiceField(field) && field.loading === true,
  );
  // A loading select is empty because options have not arrived, not because the
  // user skipped it. Treat that as a wait state so Enter does not look like a
  // required-field miss that then lingers after the default branch fills in.
  const hasEmptyNonLoadingField = fields.some(
    (field) =>
      !(isChoiceField(field) && field.loading === true) &&
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
                {field.kind === "searchable-select" ? (
                  <SearchableSelectField
                    field={field}
                    value={resolvedValues[field.name] ?? ""}
                    disabled={submitting || field.loading === true}
                    onValueChange={(value) =>
                      setValues((current) => ({
                        ...current,
                        [field.name]: value,
                      }))
                    }
                  />
                ) : field.kind === "select" ? (
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

/** True when the selected label is clipped inside the search input. */
function inputOverflows(root: HTMLElement | null): boolean {
  const input = root?.querySelector("input");
  if (!input) return false;
  return input.scrollWidth - input.clientWidth > 1;
}

/** Branch picker: the field itself is the search box, and the list hangs under it at the same width. */
function SearchableSelectField({
  field,
  value,
  disabled,
  onValueChange,
}: {
  field: SearchableSelectEntityField;
  value: string;
  disabled: boolean;
  onValueChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  const fieldRef = useRef<HTMLDivElement>(null);
  const menuOpenRef = useRef(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);
  const selected =
    field.options.find((option) => option.value === value) ?? null;

  return (
    <Popover
      open={previewOpen && !menuOpen}
      onOpenChange={(next, details) => {
        // Clicking the field opens the branch list. The preview is hover-only.
        if (details.reason === "trigger-press") {
          details.cancel();
          return;
        }
        if (
          next &&
          (menuOpenRef.current || !inputOverflows(fieldRef.current))
        ) {
          details.cancel();
          return;
        }
        setPreviewOpen(next);
      }}
    >
      <PopoverTrigger
        nativeButton={false}
        openOnHover
        delay={300}
        render={
          <div
            ref={fieldRef}
            role="presentation"
            tabIndex={-1}
            className="min-w-0 w-full"
          />
        }
      >
        <Combobox
          items={field.options}
          value={selected}
          disabled={disabled}
          isItemEqualToValue={(item, current) => item.value === current.value}
          itemToStringLabel={(item) => item.label}
          filter={(item, query) =>
            optionMatchesQuery(item, query, selected?.label ?? "")
          }
          onOpenChange={(open) => {
            menuOpenRef.current = open;
            setMenuOpen(open);
            if (open) setPreviewOpen(false);
          }}
          onValueChange={(next, details) => {
            // Emptying the box is how you search. The chosen branch stays until another is picked.
            if (details.reason === "input-clear") {
              details.cancel();
              return;
            }
            onValueChange(next?.value ?? "");
          }}
        >
          <ComboboxInput
            id={`entity-${field.name}`}
            showTrigger={false}
            placeholder={t("dialog.searchBranch")}
            aria-busy={field.loading === true}
            className="w-full bg-transparent shadow-none [&_[data-slot=input-group-addon]]:hidden [&_[data-slot=input-group-control]]:pr-8!"
            onFocus={(event) => {
              event.currentTarget.select();
            }}
          >
            <span className="pointer-events-none absolute inset-y-0 right-2 flex items-center text-muted-foreground">
              {field.loading === true ? (
                <Spinner className="size-3.5" aria-hidden="true" />
              ) : (
                <IconSearch className="size-4" aria-hidden="true" />
              )}
            </span>
          </ComboboxInput>
          <ComboboxContent
            align="start"
            sideOffset={4}
            className="w-(--anchor-width)! min-w-(--anchor-width)! max-w-(--anchor-width)! p-1"
          >
            <ComboboxEmpty>{t("dialog.noMatchingBranches")}</ComboboxEmpty>
            <ComboboxList className="max-h-52 p-0">
              {(option: SelectOption) => (
                <ComboboxItem
                  key={option.value}
                  value={option}
                  className="items-start py-1.5 pr-2 pl-2 leading-5 [&>span.absolute]:hidden"
                >
                  <span className="min-w-0 flex-1 text-left wrap-break-word whitespace-normal">
                    {option.label}
                  </span>
                </ComboboxItem>
              )}
            </ComboboxList>
          </ComboboxContent>
        </Combobox>
      </PopoverTrigger>
      {selected ? (
        <PopoverContent
          side="top"
          align="start"
          className="w-auto max-w-sm text-left wrap-break-word whitespace-normal"
        >
          {selected.label}
        </PopoverContent>
      ) : null}
    </Popover>
  );
}
