import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import type {
  PreviewWorkflowRunResumeResponse,
  ResumeRollbackMode,
} from "@ora/contracts";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Checkbox,
  RadioGroup,
  RadioGroupItem,
  Spinner,
} from "@ora/ui";
import { localizeContractError } from "../../i18n/contract-error";
import {
  usePreviewWorkflowRunResume,
  useResumeWorkflowRun,
} from "../../state/data/workflow-runs";
import { rollbackUnavailableReasonText } from "./resume-rollback";

interface ResumeRunDialogProps {
  open: boolean;
  runId: string;
  onOpenChange: (open: boolean) => void;
  onResumed: () => void;
}

const SNAPSHOT_REASON_KEYS = {
  node_missing: "workflowRun.resume.snapshotReason.node_missing",
  node_type_changed: "workflowRun.resume.snapshotReason.node_type_changed",
  start_node_changed: "workflowRun.resume.snapshotReason.start_node_changed",
  start_variables_changed:
    "workflowRun.resume.snapshotReason.start_variables_changed",
  variable_type_changed:
    "workflowRun.resume.snapshotReason.variable_type_changed",
  variable_missing: "workflowRun.resume.snapshotReason.variable_missing",
} as const;

/** Confirms how to treat the worktree before resuming a failed or cancelled run. */
export function ResumeRunDialog({
  open,
  runId,
  onOpenChange,
  onResumed,
}: ResumeRunDialogProps) {
  const { t } = useTranslation();
  const preview = usePreviewWorkflowRunResume();
  const resume = useResumeWorkflowRun();
  const [rollback, setRollback] = useState<ResumeRollbackMode>("keep");
  const [usePublished, setUsePublished] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [seedKey, setSeedKey] = useState<string | null>(null);
  const previewMutate = preview.mutate;
  const previewReset = preview.reset;
  const nextSeedKey = open ? runId : null;
  if (nextSeedKey !== null && nextSeedKey !== seedKey) {
    setSeedKey(nextSeedKey);
    setRollback("keep");
    setUsePublished(false);
    setError(null);
  }
  if (!open && seedKey !== null) {
    setSeedKey(null);
  }

  useEffect(() => {
    if (!open) {
      return;
    }
    previewReset();
    previewMutate({ runId });
  }, [open, runId, previewMutate, previewReset]);

  const previewData = preview.data;
  const checkpointReason = rollbackUnavailableReasonText(
    previewData?.checkpointUnavailableReason,
    t,
  );
  const nodeFilesReason = rollbackUnavailableReasonText(
    previewData?.nodeFilesUnavailableReason,
    t,
  );

  /** Submits the chosen rollback mode and closes only after the run actually resumes. */
  async function confirm(): Promise<void> {
    setError(null);
    try {
      await resume.mutateAsync({
        runId,
        rollback,
        ...(usePublished && previewData?.publishedSnapshotId != null
          ? { snapshotId: previewData.publishedSnapshotId }
          : {}),
      });
      onResumed();
      onOpenChange(false);
    } catch (cause) {
      setError(localizeContractError(cause, t));
    }
  }

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent className="sm:max-w-lg">
        <AlertDialogHeader>
          <AlertDialogTitle>{t("workflowRun.resume.title")}</AlertDialogTitle>
          <AlertDialogDescription>
            {t("workflowRun.resume.description")}
          </AlertDialogDescription>
        </AlertDialogHeader>

        {preview.isPending ? (
          <p className="mt-2 text-xs text-muted-foreground" role="status">
            {t("workflowRun.resume.loadingPreview")}
          </p>
        ) : null}
        {preview.isError ? (
          <p className="mt-2 text-xs text-destructive" role="alert">
            {t("workflowRun.resume.previewFailed")}
          </p>
        ) : null}

        {previewData
          ? previewData.failedNodes.map((node) => {
              const recorded = new Set(
                node.nodeFileChanges.map((change) => change.path),
              );
              const extra = node.changedSinceCheckpoint.filter(
                (change) => !recorded.has(change.path),
              ).length;
              const paths = [
                ...node.nodeFileChanges.map((change) => change.path),
                ...node.changedSinceCheckpoint
                  .map((change) => change.path)
                  .filter((path) => !recorded.has(path)),
              ];
              return (
                <div key={node.nodeRunId} className="mt-2 space-y-1.5">
                  <p className="text-xs leading-5">
                    {t("workflowRun.resume.nodeSummary", {
                      nodeId: node.nodeId,
                      nodeFiles: node.nodeFileChanges.length,
                      total: node.changedSinceCheckpoint.length,
                      extra,
                    })}
                  </p>
                  {paths.length > 0 ? (
                    <details>
                      <summary className="cursor-pointer text-xs text-muted-foreground">
                        {t("workflowRun.field.fileChanges")}
                      </summary>
                      <ul className="mt-1 max-h-24 overflow-auto text-xs">
                        {paths.map((path) => (
                          <li key={path}>{path}</li>
                        ))}
                      </ul>
                    </details>
                  ) : null}
                </div>
              );
            })
          : null}

        {previewData
          ? uniqueResumeUnits(previewData.failedNodes).map((name) => (
              <p
                key={name}
                className="mt-2 text-xs leading-5 text-muted-foreground"
              >
                {t("workflowRun.resume.compositeRestart", { name })}
              </p>
            ))
          : null}

        {previewData?.publishedSnapshotId != null &&
        previewData.publishedSnapshotId !== previewData.currentSnapshotId ? (
          <label
            className={`mt-3 flex items-start gap-2 text-sm ${
              !previewData.publishedSnapshotSwitchable ? "opacity-50" : ""
            }`}
          >
            <Checkbox
              className="mt-0.5"
              checked={usePublished}
              disabled={!previewData.publishedSnapshotSwitchable}
              onCheckedChange={(checked) => setUsePublished(checked === true)}
            />
            <span className="space-y-1">
              <span className="block">
                {t("workflowRun.resume.switchPublished", {
                  version: previewData.publishedSnapshotVersion,
                  current: previewData.currentSnapshotVersion,
                })}
              </span>
              {!previewData.publishedSnapshotSwitchable ? (
                <span className="block text-xs text-muted-foreground">
                  {snapshotReasonText(
                    previewData.publishedSnapshotIncompatibleReason,
                    t,
                  )}
                </span>
              ) : null}
            </span>
          </label>
        ) : null}

        <RadioGroup
          className="mt-3 gap-2"
          value={rollback}
          onValueChange={(value) => {
            if (
              value === "keep" ||
              value === "node_files" ||
              value === "checkpoint"
            ) {
              setRollback(value);
            }
          }}
        >
          <label className="flex items-start gap-2 text-sm">
            <RadioGroupItem value="keep" className="mt-0.5" />
            <span>{t("workflowRun.resume.keep")}</span>
          </label>
          <label
            className={`flex items-start gap-2 text-sm ${
              previewData && !previewData.nodeFilesAvailable ? "opacity-50" : ""
            }`}
          >
            <RadioGroupItem
              value="node_files"
              className="mt-0.5"
              disabled={
                previewData !== undefined && !previewData.nodeFilesAvailable
              }
            />
            <span className="space-y-1">
              <span className="block">{t("workflowRun.resume.nodeFiles")}</span>
              {nodeFilesReason ? (
                <span className="block text-xs text-muted-foreground">
                  {nodeFilesReason}
                </span>
              ) : null}
            </span>
          </label>
          <label
            className={`flex items-start gap-2 text-sm ${
              previewData && !previewData.checkpointAvailable
                ? "opacity-50"
                : ""
            }`}
          >
            <RadioGroupItem
              value="checkpoint"
              className="mt-0.5"
              disabled={
                previewData !== undefined && !previewData.checkpointAvailable
              }
            />
            <span className="space-y-1">
              <span className="block">
                {t("workflowRun.resume.checkpoint")}
              </span>
              {checkpointReason ? (
                <span className="block text-xs text-muted-foreground">
                  {checkpointReason}
                </span>
              ) : null}
            </span>
          </label>
        </RadioGroup>

        <p className="mt-2 text-xs text-muted-foreground">
          {t("workflowRun.resume.safetyNote")}
        </p>

        {error ? (
          <p className="mt-2 text-xs text-destructive" role="alert">
            {error}
          </p>
        ) : null}

        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <Button
            type="button"
            disabled={
              preview.isPending ||
              preview.isError ||
              previewData === undefined ||
              resume.isPending
            }
            onClick={() => void confirm()}
          >
            {resume.isPending ? (
              <span className="inline-flex items-center gap-1.5">
                <Spinner className="size-3.5" />
                {t("workflowRun.resume.confirm")}
              </span>
            ) : (
              t("workflowRun.resume.confirm")
            )}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** Unique composite ids that will restart from round 1. */
function uniqueResumeUnits(
  failedNodes: PreviewWorkflowRunResumeResponse["failedNodes"],
): string[] {
  const seen = new Set<string>();
  const names: string[] = [];
  for (const node of failedNodes) {
    const owner = node.resumeUnitNodeId;
    if (owner == null || owner === "" || seen.has(owner)) {
      continue;
    }
    seen.add(owner);
    names.push(owner);
  }
  return names;
}

/** Maps a machine-readable snapshot incompatibility onto the matching translated explanation. */
function snapshotReasonText(
  reason: string | null | undefined,
  t: TFunction,
): string | null {
  if (reason === null || reason === undefined || reason === "") {
    return null;
  }
  const separator = reason.indexOf(":");
  const prefix = separator === -1 ? reason : reason.slice(0, separator);
  const id = separator === -1 ? "" : reason.slice(separator + 1);
  if (prefix === "node_missing") {
    return t(SNAPSHOT_REASON_KEYS.node_missing, { id });
  }
  if (prefix === "node_type_changed") {
    return t(SNAPSHOT_REASON_KEYS.node_type_changed, { id });
  }
  if (prefix === "start_node_changed") {
    return t(SNAPSHOT_REASON_KEYS.start_node_changed);
  }
  if (prefix === "start_variables_changed") {
    return t(SNAPSHOT_REASON_KEYS.start_variables_changed);
  }
  if (prefix === "variable_type_changed") {
    return t(SNAPSHOT_REASON_KEYS.variable_type_changed, { id });
  }
  if (prefix === "variable_missing") {
    return t(SNAPSHOT_REASON_KEYS.variable_missing, { id });
  }
  return reason;
}
