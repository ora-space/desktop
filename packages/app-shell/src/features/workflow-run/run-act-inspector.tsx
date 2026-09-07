import { useTranslation } from "react-i18next";
import { Button, Input, Textarea, cn } from "@ora/ui";
import {
  IconLayoutSidebarRightCollapse,
  IconSparkles,
} from "@tabler/icons-react";
import { createMockWorkflowNodeType } from "@ora/workflow-mock";
import { formatRunClock } from "../../lib/format";
import { formatWorkflowNodeOutput } from "./format-node-output";
import {
  conditionBranchesSummary,
  createWorkflowSummaryLabels,
  getNodeMetadata,
  junctionFailureStrategyLabel,
  junctionWaitStrategyLabel,
} from "../workflow-node-chrome";
import { RunActAgentConfig } from "./run-act-agent-config";
import { RunActArtifacts } from "./run-act-artifacts";
import { RunActFileChanges } from "./run-act-file-changes";
import { RunBriefPopover } from "./run-brief-popover";
import { RunStatusBadge } from "./run-status-mark";
import { shouldPreviewBrief } from "./should-preview-brief";
import type {
  GraphWorkflowNodeState,
  GraphWorkflowSnapshotNodePatch,
  WorkflowArtifact,
  WorkflowNodeData,
  WorkflowNodeFileChange,
  WorkflowVariableValueType,
} from "@ora/workflow-runtime";

interface RunActInspectorProps {
  nodeId: string | null;
  data: WorkflowNodeData | null;
  state: GraphWorkflowNodeState | null;
  artifacts: WorkflowArtifact[];
  revealedArtifactId: string | null;
  /**
   * When true, description and a human-approval prompt are editable for this run only
   * (`pending` overrides on the frozen snapshot).
   */
  editable?: boolean;
  onPatchNode?: (patch: GraphWorkflowSnapshotNodePatch) => void;
  /**
   * When provided, the Start prompt edits a local draft instead of patching on every keystroke
   * and a save bar commits it once. `instructionDraft` stays `null` until the user types, so the
   * field falls back to the snapshot input until then.
   */
  instructionDraft?: string | null;
  variableDraft?: Record<string, unknown> | null;
  onInstructionDraftChange?: (value: string) => void;
  onVariableDraftChange?: (name: string, value: unknown) => void;
  onSaveInstruction?: () => void;
  onDiscardInstructionDraft?: () => void;
  instructionSavePending?: boolean;
  /** Fallback close action when no stage card can host the persistent toggle. */
  onClose?: () => void;
}

/** Formats an optional typed Start value for the compact read-only summary. */
function formatInputVariableValue(value: unknown): string {
  if (value === undefined || value === null) {
    return "—";
  }
  return typeof value === "string" ? value : JSON.stringify(value);
}

/** Keeps an intentionally cleared run-time value visually empty while it is being edited. */
function inputVariableDraftText(value: unknown): string {
  return value === undefined || value === null
    ? ""
    : formatInputVariableValue(value);
}

/** Parses a run-time Start value while preserving an empty field as an explicit clear. */
function parseInputVariableValue(
  text: string,
  valueType: WorkflowVariableValueType,
): unknown {
  if (text === "") {
    return null;
  }
  if (valueType === "string") {
    return text;
  }
  if (valueType === "boolean") {
    return text === "true" ? true : text === "false" ? false : text;
  }
  if (valueType === "integer" || valueType === "number") {
    const value = Number(text);
    return Number.isFinite(value) ? value : text;
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return text;
  }
}

/**
 * Theater companion rail: read-only settings-parity configuration plus
 * execution metrics and artifacts.
 */
export function RunActInspector({
  nodeId,
  data,
  state,
  artifacts,
  revealedArtifactId,
  editable = false,
  onPatchNode,
  instructionDraft,
  variableDraft,
  onInstructionDraftChange,
  onVariableDraftChange,
  onSaveInstruction,
  onDiscardInstructionDraft,
  instructionSavePending = false,
  onClose,
}: RunActInspectorProps) {
  const { t } = useTranslation();
  // The node's incremental worktree changes arrive in its run payload, captured by the engine.
  const fileChanges = state?.fileChanges ?? [];

  if (nodeId === null || data === null || state === null) {
    return (
      <aside
        className="flex min-h-0 min-w-0 flex-1 flex-col bg-background"
        aria-label={t("workflowRun.inspector.label")}
      >
        <InspectorHeader
          title={t("workflowRun.inspector.title")}
          subtitle={t("workflowRun.inspector.selectHint")}
          onClose={onClose}
        />
        <div className="flex flex-1 flex-col items-center justify-center px-6 text-center">
          <span className="mb-3 flex size-10 items-center justify-center rounded-xl bg-muted">
            <IconSparkles className="size-5 text-muted-foreground" />
          </span>
          <p className="text-xs font-medium">
            {t("workflowRun.inspector.empty")}
          </p>
          <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
            {t("workflowRun.inspector.emptyHint")}
          </p>
        </div>
      </aside>
    );
  }

  return (
    <RunActInspectorPanel
      nodeId={nodeId}
      data={data}
      state={state}
      artifacts={artifacts}
      revealedArtifactId={revealedArtifactId}
      editable={editable}
      onPatchNode={onPatchNode}
      instructionDraft={instructionDraft}
      variableDraft={variableDraft}
      onInstructionDraftChange={onInstructionDraftChange}
      onVariableDraftChange={onVariableDraftChange}
      onSaveInstruction={onSaveInstruction}
      onDiscardInstructionDraft={onDiscardInstructionDraft}
      instructionSavePending={instructionSavePending}
      fileChanges={fileChanges}
      onClose={onClose}
    />
  );
}

function RunActInspectorPanel({
  nodeId,
  data,
  state,
  artifacts,
  revealedArtifactId,
  editable,
  onPatchNode,
  instructionDraft,
  variableDraft,
  onInstructionDraftChange,
  onVariableDraftChange,
  onSaveInstruction,
  onDiscardInstructionDraft,
  instructionSavePending,
  fileChanges,
  onClose,
}: {
  nodeId: string;
  data: WorkflowNodeData;
  state: GraphWorkflowNodeState;
  artifacts: WorkflowArtifact[];
  revealedArtifactId: string | null;
  editable: boolean;
  onPatchNode?: (patch: GraphWorkflowSnapshotNodePatch) => void;
  instructionDraft?: string | null;
  variableDraft?: Record<string, unknown> | null;
  onInstructionDraftChange?: (value: string) => void;
  onVariableDraftChange?: (name: string, value: unknown) => void;
  onSaveInstruction?: () => void;
  onDiscardInstructionDraft?: () => void;
  instructionSavePending?: boolean;
  fileChanges: WorkflowNodeFileChange[];
  onClose?: () => void;
}) {
  const { i18n, t } = useTranslation();
  const locale =
    i18n.resolvedLanguage === "en-US" ? ("en-US" as const) : ("zh-CN" as const);
  const nodeType = createMockWorkflowNodeType(data.kind, locale);
  const metadata = getNodeMetadata(data.kind);
  const Icon = metadata.icon;
  const summaryLabels = createWorkflowSummaryLabels(locale);
  const toolParameters = data.toolParameters ?? [];
  const timingRange =
    state.startedAt !== undefined || state.finishedAt !== undefined
      ? [
          state.startedAt !== undefined
            ? formatRunClock(state.startedAt, locale)
            : "—",
          state.finishedAt !== undefined
            ? formatRunClock(state.finishedAt, locale)
            : "—",
        ].join(" — ")
      : null;
  const agentConfig = data.agentConfig;
  const canEdit = editable && onPatchNode !== undefined;
  const promptLabel = nodeType.configFields.includes("initialPrompt")
    ? t("settings.workflow.field.initialPrompt")
    : nodeType.configFields.includes("approvalPrompt")
      ? t("settings.workflow.field.approvalPrompt")
      : null;
  const promptValue =
    data.kind === "start" ? (data.input ?? "") : (data.instruction ?? "");

  return (
    <aside
      className="flex min-h-0 min-w-0 flex-1 flex-col bg-background"
      aria-label={t("workflowRun.inspector.label")}
    >
      <div className="border-b border-border px-4 py-3">
        <div className="flex items-center gap-2.5">
          <span
            className={cn(
              "flex size-8 shrink-0 items-center justify-center rounded-lg",
              metadata.tone,
            )}
          >
            <Icon className="size-4" />
          </span>
          <h3 className="min-w-0 flex-1 truncate font-sans text-base font-semibold">
            {data.title}
          </h3>
          <RunStatusBadge status={state.status} quiet className="shrink-0" />
          {onClose !== undefined && (
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="shrink-0 cursor-pointer"
              aria-label={t("workflowRun.inspector.collapse")}
              onClick={onClose}
            >
              <IconLayoutSidebarRightCollapse className="size-4" />
            </Button>
          )}
        </div>
        <p className="mt-1 truncate text-[11px] text-muted-foreground">
          {data.description}
        </p>
      </div>

      <div className="min-h-0 flex-1 space-y-5 overflow-y-auto p-4">
        <InspectorSection>
          {nodeType.configFields.includes("agent") &&
            agentConfig !== undefined && (
              <RunActAgentConfig config={agentConfig} />
            )}
          {data.inputVariables !== undefined &&
            data.inputVariables.length > 0 &&
            (canEdit ? (
              <div className="space-y-2">
                {data.inputVariables.map((variable) => {
                  const drafted =
                    variableDraft !== null &&
                    variableDraft !== undefined &&
                    Object.hasOwn(variableDraft, variable.name);
                  const value = drafted
                    ? variableDraft[variable.name]
                    : variable.value;
                  return (
                    <EditableField
                      key={variable.name}
                      id={`run-start-variable-${variable.name}`}
                      label={`${variable.name} (${variable.valueType})`}
                      value={inputVariableDraftText(value)}
                      onChange={(text) =>
                        onVariableDraftChange?.(
                          variable.name,
                          parseInputVariableValue(text, variable.valueType),
                        )
                      }
                    />
                  );
                })}
              </div>
            ) : (
              <ReadOnlyField
                label={t("settings.workflow.section.inputVariables")}
                value={data.inputVariables
                  .map(
                    (variable) =>
                      `${variable.name} (${variable.valueType}) = ${formatInputVariableValue(variable.value)}`,
                  )
                  .join(", ")}
                mono
              />
            ))}
          {nodeType.configFields.includes("tool") && (
            <>
              <ReadOnlyField
                label={t("settings.workflow.field.tool")}
                value={data.tool ?? "—"}
                mono
              />
              {data.operation !== undefined && data.operation !== "" && (
                <ReadOnlyField
                  label={t("settings.workflow.field.operation")}
                  value={summaryLabels.operationLabel(data.operation)}
                  mono
                />
              )}
              {toolParameters.length > 0 && (
                <ReadOnlyField
                  label={t("settings.workflow.section.parameters")}
                  value={toolParameters
                    .map((parameter) => `${parameter.key} = ${parameter.value}`)
                    .join(", ")}
                  mono
                />
              )}
            </>
          )}
          {nodeType.configFields.includes("condition") && (
            <ReadOnlyField
              label={t("settings.workflow.field.condition")}
              value={
                conditionBranchesSummary(data, summaryLabels, locale) ?? "—"
              }
              mono
            />
          )}
          {nodeType.configFields.includes("waitStrategy") &&
            data.waitStrategy !== undefined && (
              <ReadOnlyField
                label={t("settings.workflow.field.waitStrategy")}
                value={junctionWaitStrategyLabel(data.waitStrategy, t)}
                mono
              />
            )}
          {nodeType.configFields.includes("failureStrategy") &&
            data.failureStrategy !== undefined && (
              <ReadOnlyField
                label={t("settings.workflow.field.failureStrategy")}
                value={junctionFailureStrategyLabel(data.failureStrategy, t)}
                mono
              />
            )}
          {nodeType.configFields.includes("maxAttempts") &&
            data.maxAttempts !== undefined && (
              <ReadOnlyField
                label={t("settings.workflow.field.maxAttempts")}
                value={String(data.maxAttempts)}
                mono
              />
            )}
          {nodeType.configFields.includes("exitCondition") &&
            data.exitCondition !== undefined &&
            data.exitCondition !== "" && (
              <ReadOnlyField
                label={t("settings.workflow.field.exitCondition")}
                value={data.exitCondition}
                mono
              />
            )}
          {promptLabel !== null &&
            (canEdit ? (
              <div className="space-y-1.5">
                <EditableField
                  id={`run-node-instruction-${nodeId}`}
                  label={promptLabel}
                  value={instructionDraft ?? promptValue}
                  multiline
                  onChange={(value) => {
                    if (onSaveInstruction !== undefined) {
                      onInstructionDraftChange?.(value);
                    } else {
                      onPatchNode(
                        data.kind === "start"
                          ? { input: value }
                          : { instruction: value },
                      );
                    }
                  }}
                />
                {(instructionDraft !== null &&
                  instructionDraft !== undefined) ||
                variableDraft !== null ? (
                  <div className="flex items-center justify-end gap-2">
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="cursor-pointer"
                      onClick={onDiscardInstructionDraft}
                    >
                      {t("workflowRun.inspector.discardDraft")}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      className="cursor-pointer"
                      disabled={instructionSavePending}
                      onClick={onSaveInstruction}
                    >
                      {instructionSavePending
                        ? t("workflowRun.inspector.savingDraft")
                        : t("workflowRun.inspector.saveDraft")}
                    </Button>
                  </div>
                ) : null}
              </div>
            ) : (
              <ReadOnlyField
                label={promptLabel}
                value={promptValue}
                multiline
              />
            ))}
        </InspectorSection>

        <InspectorSection title={t("workflowRun.inspector.execution")}>
          {data.kind === "output" && state.output?.summary !== undefined && (
            <ReadOnlyField
              label={t("workflowRun.inspector.output")}
              value={formatWorkflowNodeOutput(state.output.summary)}
              mono
              multiline
            />
          )}
          {timingRange !== null && (
            <p className="text-[10px] tabular-nums text-muted-foreground/80">
              {timingRange}
            </p>
          )}
          {state.errorMessage !== undefined && state.errorMessage !== "" && (
            <p
              role="alert"
              className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-[11px] leading-5 text-destructive"
            >
              {state.errorMessage}
            </p>
          )}
        </InspectorSection>

        <InspectorSection title={t("workflowRun.artifacts.title")}>
          {fileChanges.length > 0 ? (
            <RunActFileChanges files={fileChanges} />
          ) : artifacts.length > 0 ? (
            <RunActArtifacts
              artifacts={artifacts}
              revealedId={revealedArtifactId}
              embedded
            />
          ) : (
            <p className="text-[11px] leading-5 text-muted-foreground">
              {t("workflowRun.artifacts.empty")}
            </p>
          )}
        </InspectorSection>
      </div>
    </aside>
  );
}

function InspectorHeader({
  title,
  subtitle,
  onClose,
}: {
  title: string;
  subtitle: string;
  onClose?: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex items-start gap-2 px-4 py-3">
      <div className="min-w-0 flex-1">
        <h3 className="font-sans text-xs font-semibold">{title}</h3>
        <p className="mt-1 text-[11px] text-muted-foreground">{subtitle}</p>
      </div>
      {onClose !== undefined && (
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          className="shrink-0 cursor-pointer"
          aria-label={t("workflowRun.inspector.collapse")}
          onClick={onClose}
        >
          <IconLayoutSidebarRightCollapse className="size-4" />
        </Button>
      )}
    </div>
  );
}

function InspectorSection({
  title,
  children,
}: {
  title?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-2.5">
      {title !== undefined && (
        <h4 className="text-[11px] font-medium uppercase tracking-[0.04em] text-muted-foreground">
          {title}
        </h4>
      )}
      <div className="space-y-2.5">{children}</div>
    </section>
  );
}

function EditableField({
  id,
  label,
  value,
  multiline = false,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  multiline?: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <div className="space-y-1">
      <label htmlFor={id} className="text-[11px] text-muted-foreground">
        {label}
      </label>
      {multiline ? (
        <Textarea
          id={id}
          value={value}
          rows={4}
          className="min-h-24 resize-y text-xs leading-5"
          onChange={(event) => onChange(event.target.value)}
        />
      ) : (
        <Input
          id={id}
          value={value}
          className="h-9 text-xs"
          onChange={(event) => onChange(event.target.value)}
        />
      )}
    </div>
  );
}

function ReadOnlyField({
  label,
  value,
  mono = false,
  multiline = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
  multiline?: boolean;
}) {
  const { t } = useTranslation();
  const trimmed = value.trim();
  const previewable = multiline && shouldPreviewBrief(trimmed);

  return (
    <div className="space-y-1">
      <p className="text-[11px] text-muted-foreground">{label}</p>
      {previewable ? (
        <RunBriefPopover
          title={label}
          body={trimmed}
          openLabel={t("workflowRun.inspector.textOpen", { field: label })}
        >
          <span
            className={cn(
              "line-clamp-4 whitespace-pre-wrap text-xs leading-5",
              mono && "font-mono text-[11px]",
            )}
          >
            {trimmed}
          </span>
        </RunBriefPopover>
      ) : (
        <div
          data-selectable
          className={cn(
            "rounded-lg border border-border/70 bg-muted/25 px-3 py-2 text-xs text-foreground/90",
            mono && "font-mono text-[11px]",
            multiline &&
              "max-h-40 overflow-y-auto whitespace-pre-wrap leading-5",
          )}
        >
          {trimmed === "" ? "—" : trimmed}
        </div>
      )}
    </div>
  );
}
