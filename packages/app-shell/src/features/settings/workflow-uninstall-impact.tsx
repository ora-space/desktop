import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { IconAlertTriangle } from "@tabler/icons-react";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import { useSkills } from "../../state/hooks/use-skills";
import {
  useWorkflowDrafts,
  useWorkflowLibrary,
} from "../../state/data/workflows";
import {
  findWorkflowDependents,
  uninstallDependencyTargets,
} from "./workflow-dependency-impact";

/**
 * Names the workflows an uninstall would leave without a dependency.
 *
 * The check reads every workflow draft on demand, so it only mounts while the uninstall
 * confirmation is open. It reports an unreadable catalog or graph as unconfirmed instead of
 * showing no impact, because silence would tell the user the removal is safe when it is not.
 */
export function WorkflowUninstallImpact({
  pluginIds,
}: {
  /** Plugin ids the pending uninstall removes; a pack passes its removable members. */
  pluginIds: readonly string[];
}) {
  const { t } = useTranslation();
  const library = useWorkflowLibrary();
  const plugins = useInstalledPlugins();
  const skills = useSkills();
  const workflowIds = useMemo(
    () => (library.data ?? []).map((workflow) => workflow.id),
    [library.data],
  );
  const drafts = useWorkflowDrafts(workflowIds);
  const pending =
    library.isPending ||
    plugins.isPending ||
    skills.isPending ||
    drafts.some((draft) => draft.isPending);
  const failed =
    library.isError ||
    plugins.isError ||
    skills.isError ||
    drafts.some((draft) => draft.isError);

  const impact = useMemo(() => {
    if (pending || failed) return null;
    const workflows = (library.data ?? []).map((summary, index) => ({
      workflowId: summary.id,
      name: summary.name,
      graph: drafts[index]?.data?.draft.graph,
    }));
    return findWorkflowDependents({
      workflows,
      plugins: plugins.data ?? [],
      skills: skills.data ?? [],
      targets: uninstallDependencyTargets(
        pluginIds,
        plugins.data ?? [],
        skills.data ?? [],
      ),
    });
  }, [
    pending,
    failed,
    library.data,
    drafts,
    pluginIds,
    plugins.data,
    skills.data,
  ]);

  if (failed) {
    return (
      <p className="text-[12px] text-muted-foreground">
        {t("settings.plugins.uninstallDependencyUnknown")}
      </p>
    );
  }
  if (impact === null) {
    return (
      <p role="status" className="text-[12px] text-muted-foreground">
        {t("settings.plugins.uninstallDependencyChecking")}
      </p>
    );
  }
  if (impact.dependents.length === 0) {
    return impact.unreadable > 0 ? (
      <p className="text-[12px] text-muted-foreground">
        {t("settings.plugins.uninstallDependencyUnreadable", {
          count: impact.unreadable,
        })}
      </p>
    ) : null;
  }

  return (
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
  );
}
