import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Input,
  Spinner,
} from "@ora/ui";
import { localizeContractError } from "../../i18n/contract-error";
import { useCreateWorkflowRun } from "../../state/hooks/use-workflow-runs";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";

interface RunWorkflowDialogProps {
  open: boolean;
  workflow: { id: string; name: string } | null;
  target: { projectId: string; workspaceId: string } | null;
  onOpenChange: (open: boolean) => void;
}

/** Creates a pending workflow run in the exact Workspace selected from the sidebar row. */
export function RunWorkflowDialog({
  open,
  workflow,
  target,
  onOpenChange,
}: RunWorkflowDialogProps) {
  const { t } = useTranslation();
  const createRun = useCreateWorkflowRun();
  const selectWorkflowRun = useWorkspaceSelectionStore(
    (state) => state.selectWorkflowRun,
  );
  const [name, setName] = useState("");
  const [attemptedSubmit, setAttemptedSubmit] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const resolvedRunName = name.trim() || (workflow?.name.trim() ?? "");
  const nameMissing = resolvedRunName === "";

  // The dialog is global, so seed each newly selected workflow without carrying a prior run name.
  const [seedKey, setSeedKey] = useState<string | null>(null);
  const nextSeedKey =
    open && workflow !== null && target !== null
      ? `${workflow.id}:${workflow.name}:${target.workspaceId}`
      : null;
  if (nextSeedKey !== null && nextSeedKey !== seedKey && workflow !== null) {
    setSeedKey(nextSeedKey);
    setName(workflow.name);
    setAttemptedSubmit(false);
    setError(null);
  }
  if (!open && seedKey !== null) {
    setSeedKey(null);
  }

  /** Persists the run against the already-selected Workspace and focuses it in the shell. */
  async function submit(): Promise<void> {
    if (workflow === null || target === null || nameMissing) {
      setAttemptedSubmit(true);
      return;
    }
    setError(null);
    try {
      const result = await createRun.mutateAsync({
        projectId: target.projectId,
        workspaceId: target.workspaceId,
        workflowId: workflow.id,
        name: resolvedRunName,
      });
      useUiStore.getState().expandProject(target.projectId);
      selectWorkflowRun(result.run.id, target.projectId);
      onOpenChange(false);
      resetLocalState();
    } catch (cause) {
      setError(resolveRunWorkflowError(cause, t));
    }
  }

  /** Clears transient form state whenever the global dialog closes. */
  function resetLocalState(): void {
    setName("");
    setAttemptedSubmit(false);
    setError(null);
  }

  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) resetLocalState();
        onOpenChange(next);
      }}
    >
      <AlertDialogContent className="sm:max-w-md">
        <AlertDialogHeader>
          <AlertDialogTitle>{t("sidebar.newWorkflow")}</AlertDialogTitle>
          <AlertDialogDescription>
            {workflow === null
              ? t("workflowRun.runPickWorkflow")
              : t("workflowRun.runInWorkspaceDescription", {
                  name: workflow.name,
                })}
          </AlertDialogDescription>
        </AlertDialogHeader>

        <div className="mt-2 space-y-1.5">
          <p className="text-xs font-medium text-muted-foreground">
            {t("workflowRun.runName")}
          </p>
          <Input
            value={name}
            onChange={(event) => setName(event.target.value)}
            aria-label={t("workflowRun.runName")}
            aria-invalid={attemptedSubmit && nameMissing}
            placeholder={
              workflow === null
                ? t("workflowRun.runNamePlaceholder")
                : t("workflowRun.runNamePlaceholderWithDefault", {
                    name: workflow.name,
                  })
            }
            disabled={workflow === null || target === null}
            onKeyDown={(event) => {
              if (event.key !== "Enter") return;
              event.preventDefault();
              void submit();
            }}
          />
          {attemptedSubmit && nameMissing && (
            <p className="text-[11px] leading-5 text-destructive" role="status">
              {t("workflowRun.runRequiredName")}
            </p>
          )}
        </div>

        {error && (
          <p className="mt-2 text-xs text-destructive" role="alert">
            {error}
          </p>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <Button
            type="button"
            disabled={
              createRun.isPending || workflow === null || target === null
            }
            onClick={() => void submit()}
          >
            {createRun.isPending ? (
              <span className="inline-flex items-center gap-1.5">
                <Spinner className="size-3.5" />
                {t("common.creating")}
              </span>
            ) : (
              t("workflowRun.createRun")
            )}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** Maps persisted workflow-run failures onto translated contract messages. */
function resolveRunWorkflowError(cause: unknown, t: TFunction): string {
  return localizeContractError(cause, t);
}
