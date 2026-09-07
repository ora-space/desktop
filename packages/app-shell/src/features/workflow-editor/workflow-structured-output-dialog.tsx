import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconBraces,
  IconCheck,
  IconCirclePlus,
  IconPencil,
  IconPlus,
  IconTimeline,
  IconTrash,
} from "@tabler/icons-react";
import {
  Button,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
  Textarea,
} from "@ora/ui";
import {
  DEFAULT_WORKFLOW_STRUCTURED_OUTPUT_SCHEMA,
  validateWorkflowStructuredOutputSchema,
  WORKFLOW_VARIABLE_VALUE_TYPES,
  type WorkflowVariableValueType,
} from "@ora/workflow-mock";

type SchemaObject = Record<string, unknown>;
type EditorMode = "visual" | "json";

interface WorkflowStructuredOutputDialogProps {
  open: boolean;
  schema: SchemaObject;
  onOpenChange: (open: boolean) => void;
  onSave: (schema: SchemaObject) => void;
}

/** Edits an Agent's structured output schema without mutating the node until Save. */
export function WorkflowStructuredOutputDialog({
  open,
  schema,
  onOpenChange,
  onSave,
}: WorkflowStructuredOutputDialogProps) {
  if (!open) return null;
  return (
    <WorkflowStructuredOutputDialogContent
      schema={schema}
      onOpenChange={onOpenChange}
      onSave={onSave}
    />
  );
}

/** Owns a fresh draft for each dialog opening so Cancel can discard every edit. */
function WorkflowStructuredOutputDialogContent({
  schema,
  onOpenChange,
  onSave,
}: Omit<WorkflowStructuredOutputDialogProps, "open">) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<EditorMode>("visual");
  const [draft, setDraft] = useState<SchemaObject>(() => cloneSchema(schema));
  const [json, setJson] = useState(() => JSON.stringify(schema, null, 2));
  const [error, setError] = useState("");

  const selectMode = (nextMode: EditorMode) => {
    if (nextMode === mode) return;
    if (mode === "json") {
      const parsed = parseObjectSchema(json);
      if (
        parsed === null ||
        !validateWorkflowStructuredOutputSchema(parsed).valid
      ) {
        setError(t("settings.workflow.structuredOutput.invalidSchema"));
        return;
      }
      setDraft(parsed);
    } else {
      setJson(JSON.stringify(draft, null, 2));
    }
    setError("");
    setMode(nextMode);
  };

  const save = () => {
    const next = mode === "json" ? parseObjectSchema(json) : draft;
    if (next === null || !validateWorkflowStructuredOutputSchema(next).valid) {
      setError(t("settings.workflow.structuredOutput.invalidSchema"));
      return;
    }
    onSave(next);
    onOpenChange(false);
  };

  const clear = () => {
    const empty = cloneSchema(DEFAULT_WORKFLOW_STRUCTURED_OUTPUT_SCHEMA);
    setDraft(empty);
    setJson(JSON.stringify(empty, null, 2));
    setError("");
  };

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[min(760px,calc(100vh-2rem))] flex-col gap-0 overflow-hidden p-0 sm:max-w-3xl">
        <DialogHeader className="px-6 pt-6 pb-3">
          <DialogTitle>
            {t("settings.workflow.structuredOutput.schemaTitle")}
          </DialogTitle>
        </DialogHeader>
        <div className="flex items-center px-6 py-2">
          <div
            className="inline-flex rounded-lg bg-muted p-0.5"
            role="tablist"
            aria-label={t("settings.workflow.structuredOutput.editorMode")}
          >
            <button
              type="button"
              role="tab"
              aria-selected={mode === "visual"}
              className="flex h-8 items-center gap-1.5 rounded-md px-3 text-xs font-medium text-muted-foreground aria-selected:bg-background aria-selected:text-blue-600 aria-selected:shadow-sm"
              onClick={() => selectMode("visual")}
            >
              <IconTimeline className="size-4" />
              Visual Editor
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={mode === "json"}
              className="flex h-8 items-center gap-1.5 rounded-md px-3 text-xs font-medium text-muted-foreground aria-selected:bg-background aria-selected:text-foreground aria-selected:shadow-sm"
              onClick={() => selectMode("json")}
            >
              <IconBraces className="size-4" />
              JSON Schema
            </button>
          </div>
        </div>
        <div className="min-h-0 flex-1 px-6 pb-4">
          {mode === "visual" ? (
            <div className="h-full overflow-auto rounded-xl bg-muted/60 p-3">
              <ObjectSchemaEditor schema={draft} onChange={setDraft} />
            </div>
          ) : (
            <Textarea
              aria-label="JSON Schema"
              className="h-full min-h-0 resize-none rounded-xl bg-muted/40 font-mono text-xs leading-5"
              value={json}
              onChange={(event) => {
                setJson(event.target.value);
                setError("");
              }}
            />
          )}
          {error !== "" && (
            <p role="alert" className="mt-2 text-xs text-destructive">
              {error}
            </p>
          )}
        </div>
        <DialogFooter className="mx-0 mb-0 shrink-0 bg-background px-6 py-4">
          <Button variant="outline" onClick={clear}>
            {t("settings.workflow.structuredOutput.clear")}
          </Button>
          <span className="mx-1 hidden h-5 w-px bg-border sm:block" />
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button onClick={save}>{t("common.save")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Shows the compact persisted-schema preview beneath the structured output switch. */
export function WorkflowStructuredOutputSummary({
  schema,
  onConfigure,
}: {
  schema: SchemaObject;
  onConfigure: () => void;
}) {
  const { t } = useTranslation();
  const properties = schemaProperties(schema);
  return (
    <div className="space-y-2 rounded-lg border border-border bg-card p-2.5">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-baseline gap-2">
          <code className="truncate text-xs font-semibold">
            structured_output
          </code>
          <span className="text-[11px] text-muted-foreground">object</span>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="h-7 shrink-0 gap-1 px-2 text-xs"
          onClick={onConfigure}
        >
          <IconPencil className="size-3.5" />
          {t("settings.workflow.structuredOutput.configure")}
        </Button>
      </div>
      {Object.keys(properties).length === 0 ? (
        <div className="rounded-md bg-muted/60 px-3 py-2.5 text-center text-xs text-muted-foreground">
          {t("settings.workflow.structuredOutput.unconfigured")}
        </div>
      ) : (
        <SchemaSummaryFields schema={schema} depth={0} />
      )}
    </div>
  );
}

/** Recursively edits object properties so nested objects remain available to the variable pool. */
function ObjectSchemaEditor({
  schema,
  onChange,
  depth = 0,
}: {
  schema: SchemaObject;
  onChange: (schema: SchemaObject) => void;
  depth?: number;
}) {
  const { t } = useTranslation();
  const properties = schemaProperties(schema);
  const required = new Set(schemaRequired(schema));
  const addField = () => {
    let index = Object.keys(properties).length + 1;
    while (`field_${index}` in properties) index += 1;
    onChange(
      replaceObjectFields(
        schema,
        {
          ...properties,
          [`field_${index}`]: { type: "string" },
        },
        required,
      ),
    );
  };

  return (
    <div
      className={
        depth === 0 ? "space-y-2" : "mt-2 space-y-2 border-l border-border pl-3"
      }
    >
      {depth === 0 && (
        <div className="flex items-center justify-between gap-3 px-1 py-0.5">
          <div className="flex items-center gap-2">
            <code className="text-xs font-semibold">structured_output</code>
            <span className="text-[11px] text-muted-foreground">object</span>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-7 gap-1 px-2 text-xs"
            onClick={addField}
          >
            <IconPlus className="size-3.5" />
            {t("settings.workflow.structuredOutput.addField")}
          </Button>
        </div>
      )}
      {Object.entries(properties).map(([name, field], index) => (
        <SchemaFieldEditor
          // The property name is editable, so its position is the stable identity during typing.
          key={index}
          name={name}
          field={field}
          required={required.has(name)}
          onChange={(nextName, nextField, nextRequired) => {
            const nextProperties: Record<string, SchemaObject> = {};
            for (const [propertyName, property] of Object.entries(properties)) {
              nextProperties[propertyName === name ? nextName : propertyName] =
                propertyName === name ? nextField : property;
            }
            const nextRequiredNames = new Set(required);
            nextRequiredNames.delete(name);
            if (nextRequired) nextRequiredNames.add(nextName);
            onChange(
              replaceObjectFields(schema, nextProperties, nextRequiredNames),
            );
          }}
          onDelete={() => {
            const nextProperties = { ...properties };
            delete nextProperties[name];
            const nextRequiredNames = new Set(required);
            nextRequiredNames.delete(name);
            onChange(
              replaceObjectFields(schema, nextProperties, nextRequiredNames),
            );
          }}
        />
      ))}
    </div>
  );
}

/** Edits one schema property and delegates child fields for object-like types. */
function SchemaFieldEditor({
  name,
  field,
  required,
  onChange,
  onDelete,
}: {
  name: string;
  field: SchemaObject;
  required: boolean;
  onChange: (name: string, field: SchemaObject, required: boolean) => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const type = schemaValueType(field);
  const childSchema = childObjectSchema(field, type);
  const canAddChild = type === "array[object]" && childSchema !== null;

  const addChild = () => {
    if (!canAddChild) return;
    const childProperties = schemaProperties(childSchema);
    let index = Object.keys(childProperties).length + 1;
    while (`field_${index}` in childProperties) index += 1;
    const nextChild = replaceObjectFields(
      childSchema,
      { ...childProperties, [`field_${index}`]: { type: "string" } },
      new Set(schemaRequired(childSchema)),
    );
    onChange(name, withChildObjectSchema(field, type, nextChild), required);
  };

  return (
    <div className="relative">
      <div
        className={
          editing
            ? "rounded-lg border border-border bg-background p-2 shadow-sm"
            : "rounded-lg px-2 py-1.5 hover:bg-background/80"
        }
      >
        <div className="flex min-w-0 items-start gap-2">
          <div className="min-w-0 flex-1">
            {editing ? (
              <div className="flex min-w-0 flex-wrap items-center gap-2">
                <Input
                  aria-label={t("settings.workflow.structuredOutput.fieldName")}
                  className="h-8 min-w-40 flex-1 text-xs"
                  value={name}
                  onChange={(event) =>
                    onChange(event.target.value.trim(), field, required)
                  }
                />
                <Select
                  value={type}
                  onValueChange={(value) =>
                    onChange(name, changeSchemaType(field, value), required)
                  }
                >
                  <SelectTrigger
                    aria-label={t(
                      "settings.workflow.structuredOutput.fieldType",
                    )}
                    className="h-8 w-40 text-xs"
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
              </div>
            ) : (
              <div className="flex min-w-0 flex-wrap items-baseline gap-2">
                <code className="truncate text-xs font-semibold">{name}</code>
                <span className="text-[11px] text-muted-foreground">
                  {type}
                </span>
                {required && (
                  <span className="text-[10px] font-medium text-orange-600">
                    {t("settings.workflow.structuredOutput.required")}
                  </span>
                )}
              </div>
            )}
            {editing ? (
              <Input
                aria-label={t(
                  "settings.workflow.structuredOutput.fieldDescription",
                )}
                className="mt-2 h-8 text-xs"
                value={
                  typeof field.description === "string" ? field.description : ""
                }
                placeholder={t(
                  "settings.workflow.structuredOutput.descriptionPlaceholder",
                )}
                onChange={(event) =>
                  onChange(
                    name,
                    withDescription(field, event.target.value),
                    required,
                  )
                }
              />
            ) : (
              typeof field.description === "string" &&
              field.description !== "" && (
                <p className="mt-0.5 truncate text-[11px] text-muted-foreground">
                  {field.description}
                </p>
              )
            )}
          </div>
          <div className="flex shrink-0 items-center gap-0.5">
            <label className="mr-1 flex items-center gap-1 text-[10px] text-muted-foreground">
              {t("settings.workflow.structuredOutput.required")}
              <Switch
                size="sm"
                checked={required}
                onCheckedChange={(checked) => onChange(name, field, checked)}
              />
            </label>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              disabled={!canAddChild}
              aria-label={t(
                "settings.workflow.structuredOutput.addChildField",
                {
                  name,
                },
              )}
              onClick={addChild}
            >
              <IconCirclePlus className="size-3.5" />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={t("settings.workflow.structuredOutput.editField", {
                name,
              })}
              onClick={() => setEditing((current) => !current)}
            >
              {editing ? (
                <IconCheck className="size-3.5" />
              ) : (
                <IconPencil className="size-3.5" />
              )}
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={t("settings.workflow.structuredOutput.deleteField", {
                name,
              })}
              onClick={onDelete}
            >
              <IconTrash className="size-3.5" />
            </Button>
          </div>
        </div>
      </div>
      {childSchema !== null && (
        <ObjectSchemaEditor
          schema={childSchema}
          depth={1}
          onChange={(nextChild) =>
            onChange(
              name,
              withChildObjectSchema(field, type, nextChild),
              required,
            )
          }
        />
      )}
    </div>
  );
}

/** Renders configured fields in the same compact hierarchy used by Dify's node panel. */
function SchemaSummaryFields({
  schema,
  depth,
}: {
  schema: SchemaObject;
  depth: number;
}) {
  const { t } = useTranslation();
  const required = new Set(schemaRequired(schema));
  return (
    <div
      className={
        depth === 0
          ? "border-l border-border pl-3"
          : "ml-2 border-l border-border pl-3"
      }
    >
      {Object.entries(schemaProperties(schema)).map(([name, field]) => {
        const type = schemaValueType(field);
        const child = childObjectSchema(field, type);
        return (
          <div key={name} className="py-1">
            <div className="flex flex-wrap items-baseline gap-2 text-xs">
              <code className="font-semibold">{name}</code>
              <span className="text-[11px] text-muted-foreground">{type}</span>
              {required.has(name) && (
                <span className="text-[10px] font-medium text-orange-600">
                  {t("settings.workflow.structuredOutput.required")}
                </span>
              )}
            </div>
            {typeof field.description === "string" &&
              field.description !== "" && (
                <p className="mt-0.5 text-[11px] text-muted-foreground">
                  {field.description}
                </p>
              )}
            {child !== null && (
              <SchemaSummaryFields schema={child} depth={depth + 1} />
            )}
          </div>
        );
      })}
    </div>
  );
}

/** Clones schemas because inspector state must not share nested objects with dialog drafts. */
function cloneSchema(schema: SchemaObject): SchemaObject {
  return structuredClone(schema);
}

/** Accepts only object-rooted schemas because `structured_output` is always an object. */
function parseObjectSchema(text: string): SchemaObject | null {
  try {
    const parsed: unknown = JSON.parse(text);
    if (isSchemaObject(parsed) && parsed.type === "object") return parsed;
  } catch {
    return null;
  }
  return null;
}

/** Narrows unknown JSON values to objects used by schema helpers. */
function isSchemaObject(value: unknown): value is SchemaObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

/** Reads schema properties defensively so imported schemas cannot crash the editor. */
function schemaProperties(schema: SchemaObject): Record<string, SchemaObject> {
  if (!isSchemaObject(schema.properties)) return {};
  return Object.fromEntries(
    Object.entries(schema.properties).filter(
      (entry): entry is [string, SchemaObject] => isSchemaObject(entry[1]),
    ),
  );
}

/** Reads the valid string entries from an object's required list. */
function schemaRequired(schema: SchemaObject): string[] {
  return Array.isArray(schema.required)
    ? schema.required.filter(
        (value): value is string => typeof value === "string",
      )
    : [];
}

/** Replaces object fields while retaining compatible schema metadata. */
function replaceObjectFields(
  schema: SchemaObject,
  properties: Record<string, SchemaObject>,
  required: Set<string>,
): SchemaObject {
  return {
    ...schema,
    type: "object",
    properties,
    required: [...required].filter((name) => name in properties),
    additionalProperties: false,
  };
}

/** Converts JSON Schema array shapes into the variable types exposed by the workflow. */
function schemaValueType(schema: SchemaObject): WorkflowVariableValueType {
  if (schema.type !== "array") {
    return WORKFLOW_VARIABLE_VALUE_TYPES.includes(
      schema.type as WorkflowVariableValueType,
    )
      ? (schema.type as WorkflowVariableValueType)
      : "any";
  }
  if (!isSchemaObject(schema.items) || typeof schema.items.type !== "string") {
    return "array";
  }
  const typedArray = `array[${schema.items.type}]` as WorkflowVariableValueType;
  return WORKFLOW_VARIABLE_VALUE_TYPES.includes(typedArray)
    ? typedArray
    : "array";
}

/** Creates the canonical JSON Schema representation for a workflow variable type. */
function schemaForType(type: WorkflowVariableValueType): SchemaObject {
  if (type === "object")
    return cloneSchema(DEFAULT_WORKFLOW_STRUCTURED_OUTPUT_SCHEMA);
  if (type === "array") return { type: "array" };
  if (type.startsWith("array[")) {
    const itemType = type.slice(6, -1);
    return {
      type: "array",
      items:
        itemType === "object"
          ? cloneSchema(DEFAULT_WORKFLOW_STRUCTURED_OUTPUT_SCHEMA)
          : { type: itemType },
    };
  }
  return { type };
}

/** Changes a field's shape while retaining its human-facing description. */
function changeSchemaType(
  schema: SchemaObject,
  value: string | null,
): SchemaObject {
  const type = WORKFLOW_VARIABLE_VALUE_TYPES.includes(
    value as WorkflowVariableValueType,
  )
    ? (value as WorkflowVariableValueType)
    : "any";
  const next = schemaForType(type);
  return typeof schema.description === "string"
    ? { ...next, description: schema.description }
    : next;
}

/** Returns the nested object represented directly or as array items. */
function childObjectSchema(
  schema: SchemaObject,
  type: WorkflowVariableValueType,
): SchemaObject | null {
  if (type === "object") return schema;
  return type === "array[object]" && isSchemaObject(schema.items)
    ? schema.items
    : null;
}

/** Replaces the correct nested object container without losing field metadata. */
function withChildObjectSchema(
  schema: SchemaObject,
  type: WorkflowVariableValueType,
  child: SchemaObject,
): SchemaObject {
  return type === "array[object]"
    ? { ...schema, items: child }
    : { ...schema, ...child };
}

/** Removes blank descriptions so the persisted schema stays concise. */
function withDescription(
  schema: SchemaObject,
  description: string,
): SchemaObject {
  if (description === "") {
    const next = { ...schema };
    delete next.description;
    return next;
  }
  return { ...schema, description };
}
