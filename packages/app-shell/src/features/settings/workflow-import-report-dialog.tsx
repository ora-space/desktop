import { useMemo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  IconAlertTriangle,
  IconCircleCheck,
  IconInfoCircle,
  IconLoader2,
} from "@tabler/icons-react";
import type { ImportedWorkflowOutcome } from "@ora/contracts";
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  cn,
} from "@ora/ui";
import { useWorkflowDrafts } from "../../state/data/workflows";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import { useSkills } from "../../state/hooks/use-skills";
import type {
  WorkflowDependency,
  WorkflowDependencyFix,
} from "../workflow-editor/workflow-transfer";
import { WorkflowTransferDependencies } from "../workflow-editor/workflow-transfer-dependencies";
import {
  buildWorkflowImportReport,
  type WorkflowImportReport,
} from "./workflow-import-report";

/** The outcomes one local `.orax` import returned, plus the archive name to show for them. */
export interface WorkflowImportReportInput {
  fileName: string;
  outcomes: readonly ImportedWorkflowOutcome[];
}

interface WorkflowImportReportDialogProps {
  input: WorkflowImportReportInput;
  onClose: () => void;
  /**
   * Resolves a dependency's fix inside the pane that opened this report. The report is a nested
   * dialog of the plugin manager, so the shared Settings deep link would be adopted only by a
   * later mount; the host navigates its own panes instead.
   */
  onResolve?: (
    dependency: WorkflowDependency,
    fix: WorkflowDependencyFix,
  ) => void;
}

/**
 * Presents authoritative import outcomes independently of the optional dependency check.
 * Refused-only packages never mount the catalog readers because no workflow needs checking.
 */
export function WorkflowImportReportDialog({
  input,
  onClose,
  onResolve,
}: WorkflowImportReportDialogProps) {
  const { t } = useTranslation();
  const imported = useMemo(
    () => input.outcomes.filter((outcome) => outcome.state === "imported"),
    [input.outcomes],
  );
  const refused = input.outcomes.filter(
    (outcome) => outcome.state === "failed",
  );

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="gap-0 p-0 sm:max-w-[620px]">
        <DialogHeader className="gap-1 px-5 pt-[18px] pb-2.5">
          <DialogTitle className="text-[15px] font-semibold tracking-[-0.01em]">
            {t("settings.plugins.importReportTitle")}
          </DialogTitle>
          <DialogDescription className="font-mono text-[12.5px] text-muted-foreground">
            {input.fileName}
          </DialogDescription>
        </DialogHeader>
        <div className="flex max-h-[60vh] flex-col gap-3.5 overflow-y-auto px-5 pt-1.5 pb-3.5">
          {imported.length > 0 && (
            <>
              <p className="text-[13px]">
                {t("settings.plugins.importReportImported", {
                  count: imported.length,
                })}
              </p>
              <WorkflowImportDependencies
                fileName={input.fileName}
                imported={imported}
                onResolve={onResolve}
              />
            </>
          )}
          {refused.length > 0 && (
            <section
              aria-label={t("settings.plugins.importReportRefusedTitle", {
                count: refused.length,
              })}
              className="overflow-hidden rounded-[10px] border border-border"
            >
              <h3 className="border-b border-border px-3 py-2 text-[13px] font-medium">
                {t("settings.plugins.importReportRefusedTitle", {
                  count: refused.length,
                })}
              </h3>
              <ul className="divide-y divide-border">
                {refused.map((outcome) => (
                  <li
                    key={outcome.sourceFile}
                    className="px-3 py-2 text-[12px]"
                  >
                    <p className="font-mono break-all">{outcome.sourceFile}</p>
                    <p className="text-muted-foreground">{outcome.reason}</p>
                  </li>
                ))}
              </ul>
            </section>
          )}
        </div>
        <DialogFooter className="mx-0 mb-0 flex-row items-center gap-2 rounded-b-xl border-t border-border bg-transparent px-5 pt-3 pb-4">
          <span className="flex-1" />
          <Button onClick={onClose}>
            {t("settings.plugins.importReportClose")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Checks dependencies without suppressing known workflow names while reads are pending or fail. */
function WorkflowImportDependencies({
  fileName,
  imported,
  onResolve,
}: {
  fileName: string;
  imported: readonly Extract<ImportedWorkflowOutcome, { state: "imported" }>[];
  onResolve: WorkflowImportReportDialogProps["onResolve"];
}) {
  const { t } = useTranslation();
  const drafts = useWorkflowDrafts(
    imported.map((outcome) => outcome.workflowId),
  );
  const plugins = useInstalledPlugins();
  const skills = useSkills();
  // A failed catalog is unknown, not an empty catalog in which every dependency is missing.
  const catalogsFailed = plugins.isError || skills.isError;
  const pending =
    drafts.some((draft) => draft.isPending) ||
    plugins.isPending ||
    skills.isPending;
  const report = useMemo(() => {
    if (pending || catalogsFailed) return null;
    const graphs = new Map<string, string>();
    imported.forEach((outcome, index) => {
      const graph = drafts[index]?.data?.draft.graph;
      if (graph !== undefined) graphs.set(outcome.workflowId, graph);
    });
    return buildWorkflowImportReport({
      fileName,
      outcomes: imported,
      graphs,
      plugins: plugins.data ?? [],
      skills: skills.data ?? [],
    });
  }, [
    pending,
    catalogsFailed,
    imported,
    drafts,
    fileName,
    plugins.data,
    skills.data,
  ]);

  return (
    <>
      {catalogsFailed ? (
        <p role="alert" className="text-[13px] text-destructive">
          {t("settings.plugins.importReportCheckFailed")}
        </p>
      ) : report === null ? (
        <p
          role="status"
          className="flex items-center gap-2 text-[13px] text-muted-foreground"
        >
          <IconLoader2 className="size-4 animate-spin" />
          {t("settings.plugins.importReportChecking")}
        </p>
      ) : (
        <ReportBanner report={report} />
      )}
      {imported.map((outcome, index) => {
        const workflow = report?.workflows[index];
        return workflow?.dependencies != null ? (
          <WorkflowTransferDependencies
            key={outcome.workflowId}
            title={outcome.name}
            dependencies={workflow.dependencies}
            variant="import"
            onResolve={onResolve}
          />
        ) : (
          <section
            key={outcome.workflowId}
            className="rounded-[10px] border border-border px-3 py-2 text-[12.5px]"
          >
            <h3 className="font-medium">{outcome.name}</h3>
            {workflow?.dependencies === null && (
              <p className="text-muted-foreground">
                {t("settings.plugins.importReportUnreadable", {
                  name: outcome.name,
                })}
              </p>
            )}
          </section>
        );
      })}
    </>
  );
}

/**
 * One readiness verdict for the whole package: missing beats unavailable beats ready, because
 * only missing references fail a run outright.
 *
 * The counts cover the workflows whose graph could be read back, so a workflow still unread keeps
 * the summary from claiming the package needs nothing or is ready: the package-wide verdict would
 * be about part of what was imported. That case gets its own notice instead.
 */
function ReportBanner({ report }: { report: WorkflowImportReport }) {
  const { t } = useTranslation();
  const { missing, unavailable, installed } = report.counts;
  const total = missing + unavailable + installed;
  const unchecked = report.workflows.filter(
    (workflow) => workflow.dependencies === null,
  ).length;
  const uncheckedNotice = unchecked > 0 && (
    <Banner tone="info">
      {t("settings.plugins.importReportUnchecked", { count: unchecked })}
    </Banner>
  );

  if (missing > 0) {
    return (
      <>
        <Banner tone="warn">
          <b className="font-semibold">
            {t("settings.plugins.importReportMissingTitle", { count: missing })}
          </b>
          {t("settings.plugins.importReportMissing")}
        </Banner>
        {uncheckedNotice}
      </>
    );
  }
  if (unavailable > 0) {
    return (
      <>
        <Banner tone="info">
          {t("settings.plugins.importReportUnavailable", {
            count: unavailable,
          })}
        </Banner>
        {uncheckedNotice}
      </>
    );
  }
  if (unchecked > 0) {
    return uncheckedNotice;
  }
  if (total === 0) {
    return (
      <Banner tone="info">
        {t("settings.plugins.importReportNoReferences")}
      </Banner>
    );
  }
  return (
    <Banner tone="ok">
      {t("settings.plugins.importReportReady", { count: installed })}
    </Banner>
  );
}

/** Readiness notice with the same tones as the editor's import preview. */
function Banner({
  tone,
  children,
}: {
  tone: "warn" | "info" | "ok";
  children: ReactNode;
}) {
  const Icon =
    tone === "warn"
      ? IconAlertTriangle
      : tone === "ok"
        ? IconCircleCheck
        : IconInfoCircle;
  return (
    <div
      role="status"
      className={cn(
        "flex gap-2.5 rounded-[9px] px-3 py-2.5 text-[12.5px] leading-normal",
        tone === "warn" && "bg-amber-500/10 text-amber-800 dark:text-amber-300",
        tone === "ok" &&
          "bg-emerald-500/10 text-emerald-800 dark:text-emerald-300",
        tone === "info" && "bg-blue-500/10 text-blue-700 dark:text-blue-300",
      )}
    >
      <Icon className="mt-0.5 size-[15px] shrink-0" />
      <div>{children}</div>
    </div>
  );
}
