import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@ora/ui";
import { useContractsClient } from "../../contracts-client-context";
import { localizeContractError } from "../../i18n/contract-error";
import { WorkflowUninstallImpact } from "./workflow-uninstall-impact";

/**
 * Ownership-aware pack uninstall confirmation: the plan's remove/preserve/already-missing
 * buckets are presented exactly as the backend computed them, so the frontend never re-derives
 * which members belong to whom.
 */
export function PackUninstallConfirm({
  packId,
  open,
  onOpenChange,
  onConfirm,
  busy,
}: {
  packId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => void;
  busy: boolean;
}) {
  const { t } = useTranslation();
  const client = useContractsClient();
  const plan = useQuery({
    queryKey: ["pack-uninstall-plan", packId],
    queryFn: () => client.plugin.packUninstallPlan({ pluginId: packId }),
    enabled: open,
  });

  const computed = plan.data?.plan;
  const removable = computed?.remove ?? [];
  const planUnavailable =
    plan.isLoading ||
    plan.isError ||
    computed === null ||
    computed === undefined;

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t("settings.plugins.packUninstallTitle")}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t("settings.plugins.packUninstallDescription")}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {open && removable.length > 0 && (
          // The plan already says which members leave; naming the workflows that bind them turns
          // a package operation into its consequence for the runs the user already built.
          <WorkflowUninstallImpact pluginIds={removable} />
        )}
        <div className="max-h-64 space-y-3 overflow-y-auto text-sm">
          {plan.isLoading && (
            <p className="text-muted-foreground">
              {t("settings.plugins.packUninstallPlanLoading")}
            </p>
          )}
          {plan.isError && (
            <p className="text-destructive">
              {t("settings.plugins.packUninstallPlanFailed")}
              <span className="mt-1 block text-muted-foreground">
                {localizeContractError(plan.error, t)}
              </span>
            </p>
          )}
          {removable.length > 0 && (
            <section>
              <h4 className="mb-1 font-medium">
                {t("settings.plugins.packUninstallRemoved")}
              </h4>
              <ul className="list-disc pl-4 text-muted-foreground">
                {removable.map((memberId) => (
                  <li key={memberId}>{memberId}</li>
                ))}
              </ul>
            </section>
          )}
          {(computed?.preserve ?? []).length > 0 && (
            <section>
              <h4 className="mb-1 font-medium">
                {t("settings.plugins.packUninstallPreserved")}
              </h4>
              <ul className="list-disc pl-4 text-muted-foreground">
                {(computed?.preserve ?? []).map((entry) => (
                  <li key={entry.memberId}>
                    {t(
                      entry.reason === "pre_existing"
                        ? "settings.plugins.packUninstallPreservedPreExisting"
                        : "settings.plugins.packUninstallPreservedVersionChanged",
                      { member: entry.memberId },
                    )}
                  </li>
                ))}
              </ul>
            </section>
          )}
          {(computed?.alreadyMissing ?? []).length > 0 && (
            <section>
              <h4 className="mb-1 font-medium">
                {t("settings.plugins.packUninstallAlreadyMissing")}
              </h4>
              <ul className="list-disc pl-4 text-muted-foreground">
                {(computed?.alreadyMissing ?? []).map((memberId) => (
                  <li key={memberId}>{memberId}</li>
                ))}
              </ul>
            </section>
          )}
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction
            disabled={busy || planUnavailable}
            onClick={(event) => {
              event.preventDefault();
              onConfirm();
            }}
          >
            {t("settings.plugins.packUninstallConfirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
