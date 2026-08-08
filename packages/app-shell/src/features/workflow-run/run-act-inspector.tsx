import { useTranslation } from "react-i18next";
import { Button, cn } from "@ora/ui";
import {
  IconLayoutSidebarRightCollapse,
  IconSparkles,
} from "@tabler/icons-react";
import { createMockWorkflowNodeType } from "@ora/workflow-mock";
import { formatRunClock } from "../../lib/format";
import {
  conditionBranchesSummary,
  createWorkflowSummaryLabels,
  getNodeMetadata,
  junctionFailureStrategyLabel,
  junctionWaitStrategyLabel,
} from "../workflow-node-chrome";
import { RunActAgentConfig } from "./run-act-agent-config";
import { RunActArtifacts } from "./run-act-artifacts";
import { RunBriefPopover } from "./run-brief-popover";
import { RunStatusBadge } from "./run-status-mark";
import { shouldPreviewBrief } from "./should-preview-brief";
import type {
  GraphWorkflowNodeState,
  WorkflowArtifact,
  WorkflowNodeData,
} from "@ora/workflow-runtime";

interface RunActInspectorProps {
  nodeId: string | null;
  data: WorkflowNodeData | null;
  state: GraphWorkflowNodeState | null;
  artifacts: WorkflowArtifact[];
  revealedArtifactId: string | null;
  onClose: () => void;
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
  onClose,
}: RunActInspectorProps) {
  const { t } = useTranslation();

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
          <p className="text-xs font-medium">{t("workflowRun.inspector.empty")}</p>
          <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
            {t("workflowRun.inspector.emptyHint")}
          </p>
        </div>
      </aside>
    );
  }

  return (
    <RunActInspectorPanel
      data={data}
      state={state}
      artifacts={artifacts}
      revealedArtifactId={revealedArtifactId}
      onClose={onClose}
    />
  );
}

function RunActInspectorPanel({
  data,
  state,
  artifacts,
  revealedArtifactId,
  onClose,
}: {
  data: WorkflowNodeData;
  state: GraphWorkflowNodeState;
  artifacts: WorkflowArtifact[];
  revealedArtifactId: string | null;
  onClose: () => void;
}) {
  const { i18n, t } = useTranslation();
  const locale = i18n.resolvedLanguage === "en-US" ? "en-US" as const : "zh-CN" as const;
  const nodeType = createMockWorkflowNodeType(data.kind, locale);
  const metadata = getNodeMetadata(data.kind);
  const Icon = metadata.icon;
  const summaryLabels = createWorkflowSummaryLabels(locale);
  const toolParameters = data.toolParameters ?? [];
  const timingRange = state.startedAt !== undefined || state.finishedAt !== undefined
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

  return (
    <aside
      className="flex min-h-0 min-w-0 flex-1 flex-col bg-background"
      aria-label={t("workflowRun.inspector.label")}
    >
      <div className="flex items-center gap-2.5 px-4 py-3">
        <span
          className={cn(
            "flex size-8 shrink-0 items-center justify-center rounded-lg",
            metadata.tone,
          )}
        >
          <Icon className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-xs font-semibold">{data.title}</h3>
          <p className="truncate text-[10px] text-muted-foreground">
            {t("workflowRun.inspector.nodeSuffix", { type: nodeType.label })}
          </p>
        </div>
        <RunStatusBadge status={state.status} quiet className="shrink-0" />
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
      </div>

      <div className="min-h-0 flex-1 space-y-5 overflow-y-auto p-4">
        <InspectorSection title={t("workflowRun.inspector.config")}>
          <ReadOnlyField label={t("settings.workflow.field.name")} value={data.title} />
          <ReadOnlyField
            label={t("settings.workflow.field.description")}
            value={data.description}
          />
          {data.inputVariables !== undefined && data.inputVariables.length > 0 && (
            <ReadOnlyField
              label={t("settings.workflow.section.inputVariables")}
              value={data.inputVariables
                .map((variable) => `${variable.name} = ${variable.defaultValue ?? ""}`)
                .join(", ")}
              mono
            />
          )}
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
              value={conditionBranchesSummary(data, summaryLabels, locale) ?? "—"}
              mono
            />
          )}
          {nodeType.configFields.includes("waitStrategy") && data.waitStrategy !== undefined && (
            <ReadOnlyField
              label={t("settings.workflow.field.waitStrategy")}
              value={junctionWaitStrategyLabel(data.waitStrategy, t)}
              mono
            />
          )}
          {nodeType.configFields.includes("failureStrategy") && data.failureStrategy !== undefined && (
            <ReadOnlyField
              label={t("settings.workflow.field.failureStrategy")}
              value={junctionFailureStrategyLabel(data.failureStrategy, t)}
              mono
            />
          )}
          {nodeType.configFields.includes("maxAttempts") && data.maxAttempts !== undefined && (
            <ReadOnlyField
              label={t("settings.workflow.field.maxAttempts")}
              value={String(data.maxAttempts)}
              mono
            />
          )}
          {nodeType.configFields.includes("exitCondition")
            && data.exitCondition !== undefined
            && data.exitCondition !== "" && (
              <ReadOnlyField
                label={t("settings.workflow.field.exitCondition")}
                value={data.exitCondition}
                mono
              />
            )}
          {nodeType.configFields.includes("agent") && agentConfig !== undefined && (
            <RunActAgentConfig config={agentConfig} />
          )}
          {nodeType.configFields.includes("instruction") && (
            <ReadOnlyField
              label={t("settings.workflow.field.instruction")}
              value={data.instruction ?? ""}
              multiline
            />
          )}
        </InspectorSection>

        <InspectorSection title={t("workflowRun.inspector.execution")}>
          {timingRange !== null && (
            <p className="text-[10px] tabular-nums text-muted-foreground/80">
              {timingRange}
            </p>
          )}
          <div className="grid grid-cols-2 gap-2">
            <MetricTile
              label={t("workflowRun.field.duration")}
              value={state.durationMs !== undefined
                ? t("workflowRun.totalsDuration", { ms: state.durationMs })
                : "—"}
            />
            <MetricTile
              label={t("workflowRun.field.tokens")}
              value={state.tokenUsage?.totalTokens !== undefined
                ? t("workflowRun.totalsTokens", {
                  count: state.tokenUsage.totalTokens,
                })
                : "—"}
            />
          </div>
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
          {artifacts.length === 0
            ? (
              <p className="text-[11px] leading-5 text-muted-foreground">
                {t("workflowRun.artifacts.empty")}
              </p>
            )
            : (
              <RunActArtifacts
                artifacts={artifacts}
                revealedId={revealedArtifactId}
                embedded
              />
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
  onClose: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex items-start gap-2 px-4 py-3">
      <div className="min-w-0 flex-1">
        <h3 className="text-xs font-semibold">{title}</h3>
        <p className="mt-1 text-[11px] text-muted-foreground">{subtitle}</p>
      </div>
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
    </div>
  );
}

function InspectorSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-2.5">
      <h4 className="text-[11px] font-medium uppercase tracking-[0.04em] text-muted-foreground">
        {title}
      </h4>
      <div className="space-y-2.5">{children}</div>
    </section>
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
      {previewable
        ? (
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
        )
        : (
          <div
            data-selectable
            className={cn(
              "rounded-lg border border-border/70 bg-muted/25 px-3 py-2 text-xs text-foreground/90",
              mono && "font-mono text-[11px]",
              multiline && "max-h-40 overflow-y-auto whitespace-pre-wrap leading-5",
            )}
          >
            {trimmed === "" ? "—" : trimmed}
          </div>
        )}
    </div>
  );
}

function MetricTile({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-lg border border-border/70 bg-muted/25 px-3 py-2">
      <p className="text-[10px] text-muted-foreground">{label}</p>
      <p className="mt-0.5 text-xs font-medium tabular-nums">{value}</p>
    </div>
  );
}
