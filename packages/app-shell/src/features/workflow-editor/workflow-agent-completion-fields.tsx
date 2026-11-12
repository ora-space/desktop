import { useState } from "react";
import { useTranslation } from "react-i18next";
import { IconPlus, IconTrash } from "@tabler/icons-react";
import { Button, Input, Switch } from "@ora/ui";
import {
  WORKFLOW_AGENT_COMPLETION_MAX_FILES,
  WORKFLOW_AGENT_COMPLETION_MAX_PATH_BYTES,
  workflowAgentCompletionPathIssue,
  type WorkflowAgentCompletionFile,
  type WorkflowAgentConfig,
} from "@ora/workflow-mock";

/**
 * Edits an Agent node's completion file declarations.
 *
 * The first added file writes `schemaVersion: 4` together with the declaration, because the
 * backend refuses `completion` on version 3 instead of silently ignoring it. Removing the last
 * declaration clears the field and returns the node to version 3 so it stays publishable to Cloud.
 * Path validity is the backend's decision; the editor keeps the author's text so a rejected graph
 * can still be corrected instead of dropping the entry.
 */
export function WorkflowAgentCompletionFields({
  config,
  onChange,
}: {
  config: WorkflowAgentConfig;
  onChange: (config: WorkflowAgentConfig) => void;
}) {
  const { t } = useTranslation();
  const files: WorkflowAgentCompletionFile[] = Array.isArray(
    config.completion?.files,
  )
    ? config.completion.files
    : [];
  // An empty row is valid while the author is typing; show its error after the first blur, while
  // every non-empty invalid path is explained immediately.
  const [touched, setTouched] = useState<ReadonlySet<number>>(new Set());

  /** Writes the whole declaration, including the version its presence requires. */
  function commit(next: WorkflowAgentCompletionFile[]): void {
    onChange({
      ...config,
      schemaVersion: next.length > 0 ? 4 : 3,
      completion: next.length > 0 ? { files: next } : undefined,
    });
  }

  /** Forgets per-row touch state after the row indexes change. */
  function resetTouched(): void {
    setTouched(new Set());
  }

  /** Replaces one file while keeping the declaration's order. */
  function updateFile(
    index: number,
    patch: Partial<WorkflowAgentCompletionFile>,
  ): void {
    commit(
      files.map((file, current) =>
        current === index ? { ...file, ...patch } : file,
      ),
    );
  }

  return (
    <fieldset className="min-w-0 space-y-1.5">
      <legend className="text-[11px] font-medium text-foreground">
        {t("settings.workflow.field.completionFiles")}
      </legend>
      <p
        id="workflow-agent-completion-hint"
        className="text-[10px] leading-relaxed text-muted-foreground"
      >
        {t("settings.workflow.completion.hint")}
      </p>
      <div className="min-w-0 divide-y overflow-hidden rounded-md border border-border">
        {files.map((file, index) => {
          const issue = workflowAgentCompletionPathIssue(file.path);
          const showIssue =
            issue !== null && (file.path !== "" || touched.has(index));
          const errorId = `workflow-agent-completion-error-${index}`;
          return (
            <div
              key={index}
              className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto_auto] items-start gap-2 px-2.5 py-2"
            >
              <div className="min-w-0 space-y-1">
                <Input
                  className="h-8 min-w-0 text-xs"
                  value={file.path}
                  aria-label={t("settings.workflow.completion.path", {
                    index: index + 1,
                  })}
                  aria-invalid={showIssue}
                  aria-describedby={
                    showIssue
                      ? `workflow-agent-completion-hint ${errorId}`
                      : "workflow-agent-completion-hint"
                  }
                  placeholder={t(
                    "settings.workflow.completion.pathPlaceholder",
                  )}
                  onBlur={() =>
                    setTouched((previous) => new Set(previous).add(index))
                  }
                  onChange={(event) =>
                    updateFile(index, { path: event.target.value })
                  }
                />
                {showIssue && (
                  <p
                    id={errorId}
                    role="alert"
                    className="text-[10px] leading-relaxed text-destructive"
                  >
                    {t(`settings.workflow.completion.pathIssue.${issue}`, {
                      max: WORKFLOW_AGENT_COMPLETION_MAX_PATH_BYTES,
                    })}
                  </p>
                )}
              </div>
              <label className="flex shrink-0 items-center gap-1.5 text-[11px] text-muted-foreground">
                <Switch
                  size="sm"
                  className="shrink-0 data-checked:bg-blue-600 hover:data-checked:bg-blue-700"
                  checked={file.mustChange === true}
                  aria-label={t("settings.workflow.completion.mustChange", {
                    index: index + 1,
                  })}
                  onCheckedChange={(checked) =>
                    updateFile(index, { mustChange: checked })
                  }
                />
                {t("settings.workflow.completion.mustChangeLabel")}
              </label>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                aria-label={t("settings.workflow.completion.remove", {
                  index: index + 1,
                })}
                onClick={() => {
                  resetTouched();
                  commit(files.filter((_, current) => current !== index));
                }}
              >
                <IconTrash />
              </Button>
            </div>
          );
        })}
        {files.length === 0 && (
          <p className="px-2.5 py-3 text-xs text-muted-foreground">
            {t("settings.workflow.completion.empty")}
          </p>
        )}
      </div>
      <Button
        type="button"
        variant="outline"
        size="sm"
        className="h-7"
        disabled={files.length >= WORKFLOW_AGENT_COMPLETION_MAX_FILES}
        onClick={() => commit([...files, { path: "" }])}
      >
        <IconPlus />
        {t("settings.workflow.completion.add")}
      </Button>
      <p className="text-[10px] text-muted-foreground">
        {t("settings.workflow.completion.count", {
          count: files.length,
          max: WORKFLOW_AGENT_COMPLETION_MAX_FILES,
        })}
      </p>
    </fieldset>
  );
}
