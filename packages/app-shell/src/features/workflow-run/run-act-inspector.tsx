import { useTranslation } from "react-i18next";
import { Button, Input, Spinner, Textarea, cn } from "@ora/ui";
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
import { resolveAgentRetryDisplay } from "./agent-config-display";
import { RunActAgentConfig } from "./run-act-agent-config";
import { RunActArtifacts } from "./run-act-artifacts";
import { RunActFailedAttempts } from "./run-act-failed-attempts";
import { RunActFileChanges } from "./run-act-file-changes";
import { RunBriefPopover } from "./run-brief-popover";
import { RunLoopRoundHistory } from "./run-loop-round-history";
import { RunRetryWaitLabel } from "./run-retry-wait-label";
import { RunStatusBadge } from "./run-status-mark";
import { shouldPreviewBrief } from "./should-preview-brief";
import { useDiagnoseWorkflowNodeFailure } from "../../state/data/workflow-runs";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useContractErrorToast } from "../../i18n/use-contract-error-toast";
import type {
  GraphWorkflowNodeState,
  GraphWorkflowRunStatus,
  GraphWorkflowRound,
  GraphWorkflowSnapshotNodePatch,
  WorkflowArtifact,
  WorkflowNodeData,
  WorkflowNodeFileChange,
  WorkflowVariableValueType,
} from "@ora/workflow-runtime";
import {
  HINT_PROMISES_INJECTION_KINDS,
  NODE_FAILURE_KINDS,
  PROMPT_INJECTED_FAILURE_KINDS,
} from "./node-failure-kinds";

const KNOWN_NODE_FAILURE_KINDS = new Set<string>(NODE_FAILURE_KINDS);

interface RunActInspectorProps {
  nodeId: string | null;
  data: WorkflowNodeData | null;
  state: GraphWorkflowNodeState | null;
  /** Per-round states of composite-region nodes, keyed by node id. */
  roundStates?: Record<string, GraphWorkflowNodeState[]>;
  /** Currently viewed round; `null` shows the node-level (latest) state. */
  selectedRound: number | null;
  onRoundChange: (round: number | null) => void;
  /** Region navigation owns round selection when false, avoiding duplicate controls. */
  showRoundSelector?: boolean;
  artifacts: WorkflowArtifact[];
  revealedArtifactId: string | null;
  loopRounds?: GraphWorkflowRound[];
  loopChildTitles?: Record<string, string>;
  selectedLoopRoundId?: string | null;
  onSelectedLoopRoundChange?: (roundId: string) => void;
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
  runStatus?: GraphWorkflowRunStatus;
  runSnapshotId?: string;
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
  roundStates,
  selectedRound,
  onRoundChange,
  showRoundSelector = true,
  artifacts,
  revealedArtifactId,
  loopRounds = [],
  loopChildTitles = {},
  selectedLoopRoundId,
  onSelectedLoopRoundChange,
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
  runStatus,
  runSnapshotId,
}: RunActInspectorProps) {
  const { t } = useTranslation();
  // Region nodes hold one state per round; the round strip lets the viewer switch rounds.
  const rounds =
    nodeId !== null && roundStates !== undefined
      ? (roundStates[nodeId] ?? [])
      : [];
  const effectiveState =
    selectedRound === null || rounds.length === 0
      ? state
      : (rounds.find((round) => round.iteration === selectedRound) ?? state);
  // The node's incremental worktree changes arrive in its run payload, captured by the engine.
  const fileChanges = effectiveState?.fileChanges ?? [];

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
      state={effectiveState ?? state}
      rounds={rounds}
      selectedRound={selectedRound}
      onRoundChange={onRoundChange}
      showRoundSelector={showRoundSelector}
      artifacts={artifacts}
      revealedArtifactId={revealedArtifactId}
      loopRounds={loopRounds}
      loopChildTitles={loopChildTitles}
      selectedLoopRoundId={selectedLoopRoundId}
      onSelectedLoopRoundChange={onSelectedLoopRoundChange}
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
      runStatus={runStatus}
      runSnapshotId={runSnapshotId}
    />
  );
}

function RunActInspectorPanel({
  nodeId,
  data,
  state,
  rounds,
  selectedRound,
  onRoundChange,
  showRoundSelector,
  artifacts,
  revealedArtifactId,
  loopRounds,
  loopChildTitles,
  selectedLoopRoundId,
  onSelectedLoopRoundChange,
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
  runStatus,
  runSnapshotId,
}: {
  nodeId: string;
  data: WorkflowNodeData;
  state: GraphWorkflowNodeState;
  rounds: GraphWorkflowNodeState[];
  selectedRound: number | null;
  onRoundChange: (round: number | null) => void;
  showRoundSelector: boolean;
  artifacts: WorkflowArtifact[];
  revealedArtifactId: string | null;
  loopRounds: GraphWorkflowRound[];
  loopChildTitles: Record<string, string>;
  selectedLoopRoundId?: string | null;
  onSelectedLoopRoundChange?: (roundId: string) => void;
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
  runStatus?: GraphWorkflowRunStatus;
  runSnapshotId?: string;
}) {
  const { i18n, t } = useTranslation();
  const runId = useWorkspaceSelectionStore((s) => s.selection.workflowRunId);
  const diagnose = useDiagnoseWorkflowNodeFailure();
  const showContractError = useContractErrorToast();
  const locale =
    i18n.resolvedLanguage === "en-US" ? ("en-US" as const) : ("zh-CN" as const);
  const nodeType = createMockWorkflowNodeType(data.kind, locale);
  const metadata = getNodeMetadata(data.kind);
  const Icon = metadata.icon;
  const summaryLabels = createWorkflowSummaryLabels(locale);
  const toolParameters = data.toolParameters ?? [];
  // A waiting retry has not started; showing a time range would suggest it is already running.
  const timingRange =
    state.status !== "retry_waiting" &&
    (state.startedAt !== undefined || state.finishedAt !== undefined)
      ? [
          state.startedAt !== undefined
            ? formatRunClock(state.startedAt, locale)
            : "—",
          state.finishedAt !== undefined
            ? formatRunClock(state.finishedAt, locale)
            : "—",
        ].join(" — ")
      : null;
  // `auto_retry` marks the retry that scheduled this row; that retry only ran if the row started.
  // A wait that ended early (cancel, run failure, app restart) must not count as a retry.
  const retryNeverStarted =
    state.autoRetry !== undefined &&
    state.startedAt === undefined &&
    state.status !== "retry_waiting";
  const startedAutoRetries =
    state.autoRetry === undefined
      ? 0
      : state.autoRetry.retry - (state.startedAt === undefined ? 1 : 0);
  const agentConfig = data.agentConfig;
  // An agent whose retry policy is on failed at once: say that this kind of failure is not
  // retried, so the policy does not look broken. Rows without the recorded flag say nothing.
  const failureKindNotRetried =
    agentConfig !== undefined &&
    resolveAgentRetryDisplay(agentConfig).kind === "enabled" &&
    state.errorDetail?.autoRetryable === false;
  // A kind that is normally injected but was recorded without injection comes from a run
  // created with failure injection off: no hint may promise the agent hears about it.
  const injectionSwitchedOff =
    state.errorDetail != null &&
    PROMPT_INJECTED_FAILURE_KINDS.has(state.errorDetail.kind) &&
    state.errorDetail.injectsPreviousFailure === false;
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
      {showRoundSelector && rounds.length > 1 && (
        <div
          className="flex items-center gap-1 overflow-x-auto border-b border-border px-3 py-2"
          role="tablist"
          aria-label={t("workflowRun.inspector.rounds")}
        >
          {rounds.map((round) => {
            const roundIndex = round.iteration ?? 0;
            const active = selectedRound === roundIndex;
            return (
              <button
                key={roundIndex}
                type="button"
                role="tab"
                aria-selected={active}
                className={cn(
                  "flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium tabular-nums transition-colors",
                  active
                    ? "bg-violet-500/15 text-violet-700 dark:text-violet-300"
                    : "text-muted-foreground hover:bg-muted",
                )}
                onClick={() => onRoundChange(roundIndex)}
              >
                R{roundIndex + 1}
                <span
                  className={cn(
                    "inline-block size-1.5 rounded-full",
                    round.status === "succeeded" && "bg-emerald-500",
                    round.status === "failed" && "bg-destructive",
                    round.status === "running" &&
                      "bg-sky-500 theater-live-breathe",
                    (round.status === "idle" ||
                      round.status === "inactive" ||
                      round.status === "cancelled") &&
                      "bg-muted-foreground/40",
                    round.status === "awaiting_input" && "bg-amber-500",
                    round.status === "retry_waiting" && "bg-orange-500",
                  )}
                />
              </button>
            );
          })}
        </div>
      )}
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
        {state.status === "retry_waiting" && state.retryWait !== undefined && (
          <p className="mt-1 text-[11px] font-medium text-orange-700 dark:text-orange-300">
            <RunRetryWaitLabel wait={state.retryWait} />
          </p>
        )}
        {state.snapshotId != null &&
          runSnapshotId != null &&
          state.snapshotId !== runSnapshotId && (
            <p className="mt-1 text-[11px] text-muted-foreground">
              {t("workflowRun.nodeFromOlderSnapshotHint")}
            </p>
          )}
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
          {nodeType.configFields.includes("maxIterations") &&
            data.loopConfig !== undefined && (
              <ReadOnlyField
                label={t("settings.workflow.field.maxIterations")}
                value={String(data.loopConfig.maxIterations)}
                mono
              />
            )}
          {nodeType.configFields.includes("loopInitialValue") &&
            data.loopConfig?.variables[0]?.initial.kind === "constant" && (
              <ReadOnlyField
                label={t("settings.workflow.field.loopInitialValue")}
                value={String(data.loopConfig.variables[0].initial.value ?? "")}
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
          {state.retryAbandoned === true && (
            <p className="text-[11px] leading-5 text-muted-foreground">
              {t("workflowRun.retry.abandoned")}
            </p>
          )}
          {state.retryAbandoned !== true &&
            state.errorMessage !== undefined &&
            state.errorMessage !== "" && (
              <div
                role="alert"
                className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-[11px] leading-5 text-destructive"
              >
                {state.errorDetail != null && (
                  <div className="mb-2 space-y-1">
                    <p className="font-medium">
                      {KNOWN_NODE_FAILURE_KINDS.has(state.errorDetail.kind)
                        ? t(`workflowRun.errorKind.${state.errorDetail.kind}`)
                        : state.errorDetail.kind}
                    </p>
                    {KNOWN_NODE_FAILURE_KINDS.has(state.errorDetail.kind) && (
                      <p>
                        {t(
                          injectionSwitchedOff &&
                            HINT_PROMISES_INJECTION_KINDS.has(
                              state.errorDetail.kind,
                            )
                            ? `workflowRun.errorHintWithoutInjection.${state.errorDetail.kind}`
                            : `workflowRun.errorHint.${state.errorDetail.kind}`,
                        )}
                      </p>
                    )}
                    <p>
                      {t("workflowRun.errorAttempt", {
                        count: state.errorDetail.attempt,
                      })}
                    </p>
                    {failureKindNotRetried && (
                      <p>{t("workflowRun.retry.kindNotRetried")}</p>
                    )}
                    {state.errorDetail.resumable === false &&
                      state.errorDetail.injectsPreviousFailure === true && (
                        <p>{t("workflowRun.errorInjectedResumeHint")}</p>
                      )}
                    {injectionSwitchedOff && (
                      <p>{t("workflowRun.errorResumeWithoutInjectionHint")}</p>
                    )}
                    {state.errorDetail.resumable === false &&
                      state.errorDetail.injectsPreviousFailure !== true &&
                      !injectionSwitchedOff && (
                        <p>{t("workflowRun.errorNotResumableHint")}</p>
                      )}
                  </div>
                )}
                <p>{state.errorMessage}</p>
              </div>
            )}
          {state.status === "failed" && startedAutoRetries > 0 && (
            <p className="text-[11px] font-medium text-orange-700 dark:text-orange-300">
              {t("workflowRun.retry.exhausted", {
                count: startedAutoRetries,
              })}
            </p>
          )}
          {retryNeverStarted && state.retryAbandoned !== true && (
            <p className="text-[11px] leading-5 text-muted-foreground">
              {t("workflowRun.retry.notStarted")}
            </p>
          )}
          {(state.status === "failed" || state.status === "cancelled") &&
            (runStatus === "failed" || runStatus === "cancelled") && (
              <p className="text-[11px] text-muted-foreground">
                {t("workflowRun.resumeFromTopHint")}
              </p>
            )}
          {state.injectedFailureContext !== undefined &&
            state.injectedFailureContext !== "" && (
              <details>
                <summary>{t("workflowRun.injectedFailure.title")}</summary>
                <pre className="whitespace-pre-wrap text-[11px] leading-5">
                  {state.injectedFailureContext}
                </pre>
              </details>
            )}
          {state.status === "failed" &&
          data.kind === "agent" &&
          runId != null ? (
            <div className="space-y-2">
              {state.aiDiagnosis != null ? (
                <div className="rounded-lg border border-border px-3 py-2">
                  <h5 className="text-[11px] font-medium">
                    {t("workflowRun.aiDiagnosis.title", {
                      model: state.aiDiagnosis.model,
                    })}
                  </h5>
                  <p className="mt-1 whitespace-pre-wrap text-[11px] leading-5">
                    {state.aiDiagnosis.text}
                  </p>
                  <p className="mt-2 text-[10px] text-muted-foreground">
                    {t("workflowRun.aiDiagnosis.disclaimer")}
                  </p>
                </div>
              ) : null}
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="cursor-pointer"
                disabled={diagnose.isPending}
                onClick={() => {
                  diagnose.mutate(
                    { runId, nodeId },
                    { onError: (error) => showContractError(error) },
                  );
                }}
              >
                {diagnose.isPending ? (
                  <>
                    <Spinner className="size-3.5" />
                    {t("workflowRun.aiDiagnosis.running")}
                  </>
                ) : state.aiDiagnosis != null ? (
                  t("workflowRun.aiDiagnosis.rerun")
                ) : (
                  t("workflowRun.aiDiagnosis.run")
                )}
              </Button>
            </div>
          ) : null}
        </InspectorSection>

        <RunActFailedAttempts attempts={state.failedAttempts} />

        {data.kind === "loop" && (
          <InspectorSection title={t("workflowRun.loopRounds.title")}>
            <RunLoopRoundHistory
              rounds={loopRounds}
              nodeTitles={loopChildTitles}
              selectedRoundId={selectedLoopRoundId}
              onSelectedRoundChange={onSelectedLoopRoundChange}
            />
          </InspectorSection>
        )}

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
