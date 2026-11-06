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
  onResolve: (
    dependency: WorkflowDependency,
    fix: WorkflowDependencyFix,
  ) => void;
}

/**
 * Post-import surface for the workflow documents a plugin package carried.
 *
 * The backend installs the package and reports one outcome per document; it deliberately carries
 * no MCP/Skill identity, so this dialog reads each created workflow's graph back through the
 * workflow data owner and resolves the references with the same collector the editor's import
 * preview uses. Both import entries therefore agree on readiness, wording, and install entries,
 * and a package whose dependencies are missing no longer reports a bare success.
 */
export function WorkflowImportReportDialog({
  input,
  onClose,
  onResolve,
}: WorkflowImportReportDialogProps) {
  const { t } = useTranslation();
  const imported = useMemo(
    () =>
      input.outcomes.filter(
        (
          outcome,
        ): outcome is Extract<ImportedWorkflowOutcome, { state: "imported" }> =>
          outcome.state === "imported",
      ),
    [input.outcomes],
  );
  const drafts = useWorkflowDrafts(
    imported.map((outcome) => outcome.workflowId),
  );
  const plugins = useInstalledPlugins();
  const skills = useSkills();
  // Readiness compares the imported graphs against installed plugins and Skills, so the report
  // stays pending until both the graphs and those catalogs are there. A catalog that failed to
  // load must keep the surface from claiming anything: an empty list would read as "nothing is
  // installed", which is a different and wrong verdict.
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
      fileName: input.fileName,
      outcomes: input.outcomes,
      graphs,
      plugins: plugins.data ?? [],
      skills: skills.data ?? [],
    });
  }, [
    pending,
    catalogsFailed,
    imported,
    drafts,
    input.fileName,
    input.outcomes,
    plugins.data,
    skills.data,
  ]);

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
          {catalogsFailed ? (
            <p role="alert" className="text-[13px] text-destructive">
              {t("settings.plugins.importReportCheckFailed")}
            </p>
          ) : report === null ? (
            <p className="flex items-center gap-2 text-[13px] text-muted-foreground">
              <IconLoader2 className="size-4 animate-spin" />
              {t("settings.plugins.importReportChecking")}
            </p>
          ) : (
            <>
              {/* A package whose documents were all refused created no workflow, so there is
                  no import count or readiness verdict to report — only what was refused. */}
              {report.workflows.length > 0 && (
                <>
                  <p className="text-[13px]">
                    {t("settings.plugins.importReportImported", {
                      count: report.workflows.length,
                    })}
                  </p>
                  <ReportBanner report={report} />
                </>
              )}
              {report.workflows.map((workflow) =>
                workflow.dependencies === null ? (
                  <p
                    key={workflow.workflowId}
                    className="rounded-[10px] border border-border px-3 py-2 text-[12.5px] text-muted-foreground"
                  >
                    {t("settings.plugins.importReportUnreadable", {
                      name: workflow.name,
                    })}
                  </p>
                ) : (
                  <WorkflowTransferDependencies
                    key={workflow.workflowId}
                    title={workflow.name}
                    dependencies={workflow.dependencies}
                    variant="import"
                    onResolve={onResolve}
                  />
                ),
              )}
              {report.refused.length > 0 && (
                <section
                  aria-label={t("settings.plugins.importReportRefusedTitle", {
                    count: report.refused.length,
                  })}
                  className="overflow-hidden rounded-[10px] border border-border"
                >
                  <h3 className="border-b border-border px-3 py-2 text-[13px] font-medium">
                    {t("settings.plugins.importReportRefusedTitle", {
                      count: report.refused.length,
                    })}
                  </h3>
                  <ul className="divide-y divide-border">
                    {report.refused.map((refused) => (
                      <li
                        key={refused.sourceFile}
                        className="px-3 py-2 text-[12px]"
                      >
                        <p className="font-mono break-all">
                          {refused.sourceFile}
                        </p>
                        <p className="text-muted-foreground">
                          {refused.reason}
                        </p>
                      </li>
                    ))}
                  </ul>
                </section>
              )}
            </>
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
