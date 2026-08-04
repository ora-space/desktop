import { useTranslation } from "react-i18next";
import {
  Button,
  Input,
  Textarea,
  cn,
} from "@ora/ui";
import {
  IconLayoutSidebarRightCollapse,
  IconSparkles,
} from "@tabler/icons-react";
import {
  createMockWorkflowNodeType,
} from "@ora/workflow-mock";
import { formatRunClock } from "../../lib/format";
import { getNodeMetadata } from "../workflow-node-chrome";
import { RunActArtifacts } from "./run-act-artifacts";
import { RunStatusBadge } from "./run-status-mark";
import type {
  GraphWorkflowNodeState,
  GraphWorkflowSnapshotNodePatch,
  WorkflowArtifact,
  WorkflowNodeData,
} from "@ora/workflow-runtime";

interface RunActInspectorProps {
  nodeId: string | null;
  data: WorkflowNodeData | null;
  state: GraphWorkflowNodeState | null;
  artifacts: WorkflowArtifact[];
  revealedArtifactId: string | null;
  /**
   * When true, description / instruction are editable for this run only
   * (`pending` overrides on the frozen snapshot).
   */
  editable?: boolean;
  onPatchNode?: (patch: GraphWorkflowSnapshotNodePatch) => void;
  onClose: () => void;
}

/**
 * Theater companion rail: settings-parity fields.
 * Editable only while the host marks the run as pending (snapshot overrides).
 */
export function RunActInspector({
  nodeId,
  data,
  state,
  artifacts,
  revealedArtifactId,
  editable = false,
  onPatchNode,
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
      nodeId={nodeId}
      data={data}
      state={state}
      artifacts={artifacts}
      revealedArtifactId={revealedArtifactId}
      editable={editable}
      onPatchNode={onPatchNode}
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
  onClose,
}: {
  nodeId: string;
  data: WorkflowNodeData;
  state: GraphWorkflowNodeState;
  artifacts: WorkflowArtifact[];
  revealedArtifactId: string | null;
  editable: boolean;
  onPatchNode?: (patch: GraphWorkflowSnapshotNodePatch) => void;
  onClose: () => void;
}) {
  const { i18n, t } = useTranslation();
  const locale = i18n.resolvedLanguage === "en-US" ? "en-US" as const : "zh-CN" as const;
  const nodeType = createMockWorkflowNodeType(data.kind, locale);
  const metadata = getNodeMetadata(data.kind);
  const Icon = metadata.icon;
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
  const canEdit = editable && onPatchNode !== undefined;

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
            {canEdit
              ? t("workflowRun.inspector.nodeSuffixEditable", {
                type: nodeType.label,
              })
              : t("workflowRun.inspector.nodeSuffix", { type: nodeType.label })}
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
        {canEdit && (
          <p className="rounded-lg border border-sky-500/25 bg-sky-500/5 px-3 py-2 text-[11px] leading-5 text-sky-900 dark:text-sky-200">
            {t("workflowRun.inspector.runOnlyHint")}
          </p>
        )}

        <InspectorSection title={t("workflowRun.inspector.config")}>
          <ReadOnlyField label={t("settings.workflow.field.name")} value={data.title} />
          {canEdit
            ? (
              <EditableField
                id={`run-node-description-${nodeId}`}
                label={t("settings.workflow.field.description")}
                value={data.description}
                onChange={(value) => onPatchNode({ description: value })}
              />
            )
            : (
              <ReadOnlyField
                label={t("settings.workflow.field.description")}
                value={data.description}
              />
            )}
          {nodeType.configFields.includes("model") && (
            <ReadOnlyField
              label={t("settings.workflow.field.model")}
              value={data.model ?? "—"}
              mono
            />
          )}
          {nodeType.configFields.includes("tool") && (
            <ReadOnlyField
              label={t("settings.workflow.field.tool")}
              value={data.tool ?? "—"}
              mono
            />
          )}
          {nodeType.configFields.includes("condition") && (
            <ReadOnlyField
              label={t("settings.workflow.field.condition")}
              value={data.condition ?? "—"}
              mono
            />
          )}
          {nodeType.configFields.includes("instruction") && (
            canEdit
              ? (
                <EditableField
                  id={`run-node-instruction-${nodeId}`}
                  label={t("settings.workflow.field.instruction")}
                  value={data.instruction ?? ""}
                  multiline
                  onChange={(value) => onPatchNode({ instruction: value })}
                />
              )
              : (
                <ReadOnlyField
                  label={t("settings.workflow.field.instruction")}
                  value={data.instruction ?? ""}
                  multiline
                />
              )
          )}
          <ReadOnlyField
            label={t("workflowRun.theater.nodeId")}
            value={nodeId}
            mono
          />
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
  return (
    <div className="space-y-1">
      <p className="text-[11px] text-muted-foreground">{label}</p>
      <div
        data-selectable
        className={cn(
          "rounded-lg border border-border/70 bg-muted/25 px-3 py-2 text-xs text-foreground/90",
          mono && "font-mono text-[11px]",
          multiline && "max-h-40 overflow-y-auto whitespace-pre-wrap leading-5",
        )}
      >
        {value === "" ? "—" : value}
      </div>
    </div>
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
      {multiline
        ? (
          <Textarea
            id={id}
            value={value}
            rows={4}
            className="min-h-24 resize-y text-xs leading-5"
            onChange={(event) => onChange(event.target.value)}
          />
        )
        : (
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
