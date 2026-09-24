import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Input, Label, Switch } from "@ora/ui";
import {
  DEFAULT_WORKFLOW_AGENT_RETRY,
  WORKFLOW_AGENT_RETRY_BOUNDS,
  WORKFLOW_AGENT_RETRY_MAX_DELAY_SECONDS,
  WORKFLOW_AGENT_RETRY_NUMERIC_FIELDS,
  validateWorkflowAgentRetry,
  type WorkflowAgentConfig,
  type WorkflowAgentRetryNumericField,
  type WorkflowAgentRetryNumericIssue,
  type WorkflowAgentRetryPolicy,
} from "@ora/workflow-mock";
import { InspectorField } from "./workflow-node-details";

const FIELD_LABEL_KEYS = {
  maxRetries: "settings.workflow.field.retryMaxRetries",
  initialDelaySeconds: "settings.workflow.field.retryInitialDelay",
} as const satisfies Record<WorkflowAgentRetryNumericField, string>;

const FIELD_INPUT_IDS = {
  maxRetries: "workflow-agent-retry-max-retries",
  initialDelaySeconds: "workflow-agent-retry-initial-delay",
} as const satisfies Record<WorkflowAgentRetryNumericField, string>;

/**
 * Edits an Agent node's automatic retry policy.
 *
 * A node without `retry` shows the default policy and keeps the field absent until the author
 * changes something; any change then writes the complete policy the backend requires. Numbers
 * that fail validation stay in the input with a message and are never written to the graph, and
 * a stored number the engine would reject is replaced by its default on the next write, so
 * autosave only ever persists a policy the engine accepts.
 */
export function WorkflowAgentRetryFields({
  config,
  onChange,
}: {
  config: WorkflowAgentConfig;
  onChange: (config: WorkflowAgentConfig) => void;
}) {
  const { t } = useTranslation();
  // Imported files may carry anything here; read it as data rather than trusting the type.
  const stored: unknown = config.retry;
  const absent = stored === undefined || stored === null;
  const record = isRecord(stored) ? stored : null;
  const enabled =
    typeof record?.enabled === "boolean"
      ? record.enabled
      : DEFAULT_WORKFLOW_AGENT_RETRY.enabled;

  /** The persisted value, or the default when the policy is absent or not an object. */
  function storedValue(field: WorkflowAgentRetryNumericField): unknown {
    return record === null
      ? DEFAULT_WORKFLOW_AGENT_RETRY[field]
      : record[field];
  }

  const storedNumbers: Record<WorkflowAgentRetryNumericField, unknown> = {
    maxRetries: storedValue("maxRetries"),
    initialDelaySeconds: storedValue("initialDelaySeconds"),
  };
  // Text the author typed that failed validation, so the graph does not hold it.
  const [drafts, setDrafts] = useState<
    Partial<Record<WorkflowAgentRetryNumericField, string>>
  >({});
  const [seenNumbers, setSeenNumbers] = useState(storedNumbers);
  const changedFields = WORKFLOW_AGENT_RETRY_NUMERIC_FIELDS.filter(
    (field) => !Object.is(seenNumbers[field], storedNumbers[field]),
  );
  // React permits guarded render-time adjustment for state tied to a prop. Once the stored number
  // changes (an edit, undo, redo), text typed over the old value is discarded for good, so a later
  // redo that restores that value cannot bring the stale text back.
  if (changedFields.length > 0) {
    setSeenNumbers(storedNumbers);
    setDrafts((previous) => {
      const next = { ...previous };
      for (const field of changedFields) {
        delete next[field];
      }
      return next;
    });
  }

  function currentValue(field: WorkflowAgentRetryNumericField): unknown {
    const draft = drafts[field];
    return draft === undefined ? storedValue(field) : parseRetryNumber(draft);
  }

  const candidate = {
    enabled,
    maxRetries: currentValue("maxRetries"),
    initialDelaySeconds: currentValue("initialDelaySeconds"),
  };
  const numericIssues = validateWorkflowAgentRetry(candidate);
  // Shape problems of the stored object are fixed by the next edit, which writes a full policy.
  const malformed =
    !absent &&
    validateWorkflowAgentRetry(stored).some(
      (issue) => issue.field === null || issue.field === "enabled",
    );

  /**
   * The number a write keeps for a field the author did not just edit: the stored value when the
   * engine accepts it, otherwise the default.
   */
  function committedNumber(field: WorkflowAgentRetryNumericField): number {
    const value = storedValue(field);
    const accepted = validateWorkflowAgentRetry({
      ...DEFAULT_WORKFLOW_AGENT_RETRY,
      [field]: value,
    }).every((issue) => issue.field !== field);
    return accepted && typeof value === "number"
      ? value
      : DEFAULT_WORKFLOW_AGENT_RETRY[field];
  }

  /** Writes the complete policy; untouched fields keep their accepted stored (or default) values. */
  function commit(patch: Partial<WorkflowAgentRetryPolicy>): void {
    onChange({
      ...config,
      retry: {
        enabled,
        maxRetries: committedNumber("maxRetries"),
        initialDelaySeconds: committedNumber("initialDelaySeconds"),
        ...patch,
      },
    });
  }

  /** Commits a valid number, or keeps the typed text visible beside its validation message. */
  function editNumber(field: WorkflowAgentRetryNumericField, text: string) {
    const value = parseRetryNumber(text);
    const valid = validateWorkflowAgentRetry({
      ...candidate,
      [field]: value,
    }).every((issue) => issue.field !== field);
    if (valid && value !== undefined) {
      setDrafts((previous) => {
        const next = { ...previous };
        delete next[field];
        return next;
      });
      commit(
        field === "maxRetries"
          ? { maxRetries: value }
          : { initialDelaySeconds: value },
      );
      return;
    }
    setDrafts((previous) => ({ ...previous, [field]: text }));
  }

  /** Localizes one numeric-field issue with the field's own label and bounds. */
  function issueMessage({ field, reason }: WorkflowAgentRetryNumericIssue) {
    const label = t(FIELD_LABEL_KEYS[field]);
    switch (reason) {
      case "missing":
        return t("settings.workflow.retry.issue.missing", { field: label });
      case "notInteger":
        return t("settings.workflow.retry.issue.notInteger", { field: label });
      case "outOfRange":
        return t("settings.workflow.retry.issue.outOfRange", {
          field: label,
          min: WORKFLOW_AGENT_RETRY_BOUNDS[field].min,
          max: WORKFLOW_AGENT_RETRY_BOUNDS[field].max,
        });
    }
  }

  return (
    <div className="min-w-0 space-y-2">
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor="workflow-agent-retry" className="text-[11px]">
          {t("settings.workflow.field.retry")}
        </Label>
        <Switch
          id="workflow-agent-retry"
          className="shrink-0 data-checked:bg-blue-600 hover:data-checked:bg-blue-700"
          checked={enabled}
          aria-describedby="workflow-agent-retry-hint"
          onCheckedChange={(next) => commit({ enabled: next })}
        />
      </div>
      <ul
        id="workflow-agent-retry-hint"
        className="list-disc space-y-0.5 pl-3.5 text-[10px] leading-relaxed text-muted-foreground"
      >
        <li>{t("settings.workflow.retry.hintFailures")}</li>
        <li>{t("settings.workflow.retry.hintFeedback")}</li>
        <li>
          {t("settings.workflow.retry.hintBackoff", {
            max: WORKFLOW_AGENT_RETRY_MAX_DELAY_SECONDS,
          })}
        </li>
        <li>{t("settings.workflow.retry.hintInteractive")}</li>
      </ul>
      {malformed && (
        <p role="alert" className="text-[11px] text-destructive">
          {t("settings.workflow.retry.issue.malformed")}
        </p>
      )}
      <div className="grid grid-cols-2 items-start gap-2">
        {WORKFLOW_AGENT_RETRY_NUMERIC_FIELDS.map((field) => {
          const draft = drafts[field];
          const issue = numericIssues.find(
            (
              candidateIssue,
            ): candidateIssue is WorkflowAgentRetryNumericIssue =>
              candidateIssue.field === field,
          );
          const errorId = `${FIELD_INPUT_IDS[field]}-error`;
          return (
            <InspectorField
              key={field}
              label={t(FIELD_LABEL_KEYS[field])}
              htmlFor={FIELD_INPUT_IDS[field]}
            >
              <Input
                id={FIELD_INPUT_IDS[field]}
                type="number"
                inputMode="numeric"
                min={WORKFLOW_AGENT_RETRY_BOUNDS[field].min}
                max={WORKFLOW_AGENT_RETRY_BOUNDS[field].max}
                step={1}
                className="h-8"
                disabled={!enabled}
                value={draft ?? formatStoredNumber(storedValue(field))}
                aria-invalid={issue !== undefined}
                aria-describedby={issue === undefined ? undefined : errorId}
                onChange={(event) => editNumber(field, event.target.value)}
              />
              {/* `status` like the other inline field errors: an alert would interrupt the
                  screen reader on every keystroke, and `aria-describedby` already links it. */}
              {issue !== undefined && (
                <p
                  id={errorId}
                  role="status"
                  className="text-[11px] text-destructive"
                >
                  {issueMessage(issue)}
                </p>
              )}
            </InspectorField>
          );
        })}
      </div>
    </div>
  );
}

/** Empty input means "no value"; anything else goes through `Number` so validation can judge it. */
function parseRetryNumber(text: string): number | undefined {
  const trimmed = text.trim();
  return trimmed === "" ? undefined : Number(trimmed);
}

/** Shows whatever is stored so a malformed value stays visible next to its message. */
function formatStoredNumber(value: unknown): string {
  return value === undefined || value === null ? "" : String(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
