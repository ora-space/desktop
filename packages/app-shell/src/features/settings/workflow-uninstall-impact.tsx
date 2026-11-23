import { useTranslation } from "react-i18next";
import { IconAlertTriangle, IconInfoCircle } from "@tabler/icons-react";
import type { WorkflowUninstallImpactState } from "./use-workflow-uninstall-impact";

/**
 * Presents the workflows an uninstall would leave without a dependency, as well as surviving
 * workflow copies originally delivered by a workflow plugin.
 *
 * The dialog passes the resolved state from {@link useWorkflowUninstallImpact} so the same scan
 * both names affected workflows and disables destructive confirmation until checking finishes.
 */
export function WorkflowUninstallImpact({
  state,
}: {
  state: WorkflowUninstallImpactState;
}) {
  const { t } = useTranslation();

  if (state.status === "idle") return null;

  if (state.status === "failed") {
    // A workflow plugin uninstall must never read an unreadable library as "no imported
    // workflows": the retention notice names exactly what stays unknown.
    return (
      <div className="space-y-2.5">
        {state.dependencyImpactUnknown && (
          <p className="text-[12px] text-muted-foreground">
            {t("settings.plugins.uninstallDependencyUnknown")}
          </p>
        )}
        {state.retentionUnknown && (
          <p className="text-[12px] text-muted-foreground">
            {t("settings.plugins.uninstallImportedUnknown")}
          </p>
        )}
      </div>
    );
  }
  if (state.status === "checking") {
    return (
      <p role="status" className="text-[12px] text-muted-foreground">
        {t(
          state.scope === "imported"
            ? "settings.plugins.uninstallImportedChecking"
            : "settings.plugins.uninstallDependencyChecking",
        )}
      </p>
    );
  }

  const { impact, importedWorkflows } = state;
  const hasDependencyWarning =
    impact.dependents.length > 0 || impact.unreadable > 0;
  const hasImportedWorkflows = importedWorkflows.length > 0;
  if (!hasDependencyWarning && !hasImportedWorkflows) return null;

  return (
    <div className="space-y-2.5">
      {impact.dependents.length === 0 ? (
        impact.unreadable > 0 ? (
          <p className="text-[12px] text-muted-foreground">
            {t("settings.plugins.uninstallDependencyUnreadable", {
              count: impact.unreadable,
            })}
          </p>
        ) : null
      ) : (
        <div
          role="status"
          className="rounded-[9px] bg-amber-500/10 px-3 py-2.5 text-[12.5px] leading-normal text-amber-800 dark:text-amber-300"
        >
          <p className="flex gap-2.5 font-medium">
            <IconAlertTriangle
              aria-hidden
              className="mt-0.5 size-[15px] shrink-0"
            />
            {t("settings.plugins.uninstallDependencyTitle", {
              count: impact.dependents.length,
            })}
          </p>
          <p className="mt-1">
            {t("settings.plugins.uninstallDependencyDescription")}
          </p>
          <ul className="mt-1.5 space-y-0.5">
            {impact.dependents.map((dependent) => (
              <li key={dependent.workflowId}>
                <span className="font-medium">{dependent.name}</span>
                {dependent.nodeTitles.length > 0 && (
                  <span className="text-amber-800/80 dark:text-amber-300/80">
                    {" · "}
                    {t("settings.workflow.transfer.usedBy", {
                      nodes: dependent.nodeTitles.join(
                        t("settings.workflow.transfer.listSeparator"),
                      ),
                    })}
                  </span>
                )}
              </li>
            ))}
          </ul>
          {impact.unreadable > 0 && (
            <p className="mt-1.5">
              {t("settings.plugins.uninstallDependencyUnreadable", {
                count: impact.unreadable,
              })}
            </p>
          )}
        </div>
      )}
      {hasImportedWorkflows && (
        <div
          role="status"
          className="rounded-[9px] bg-muted/60 px-3 py-2.5 text-[12.5px] leading-normal text-foreground"
        >
          <p className="flex gap-2.5 font-medium">
            <IconInfoCircle
              aria-hidden
              className="mt-0.5 size-[15px] shrink-0 text-muted-foreground"
            />
            {t("settings.plugins.uninstallImportedTitle", {
              count: importedWorkflows.length,
            })}
          </p>
          <p className="mt-1 text-muted-foreground">
            {t("settings.plugins.uninstallImportedDescription")}
          </p>
          <ul className="mt-1.5 space-y-0.5">
            {importedWorkflows.map((workflow) => (
              <li key={workflow.workflowId} className="font-medium">
                {workflow.name}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
