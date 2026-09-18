import type { InstallOutcome } from "@ora/contracts";
import { toast } from "@ora/ui";
import type { TFunction } from "i18next";

type InstallSuccessKey =
  "settings.plugins.installSuccess" | "settings.plugins.importSuccess";

/**
 * Presents one install outcome consistently across every marketplace entry point.
 *
 * `successDescription` adds entry-point specific context to the plain success toast — a local
 * import reports the workflow documents the package carried. Outcomes that already own their
 * wording (a command conflict, a pack member journal) ignore it, so a caller never overrides
 * the message its own outcome requires.
 */
export function showPluginInstallOutcome(
  outcome: InstallOutcome,
  t: TFunction,
  successKey: InstallSuccessKey = "settings.plugins.installSuccess",
  successDescription?: string,
): void {
  if (outcome.state === "installed_with_command_conflict") {
    toast.success(
      t("settings.plugins.installCommandConflict", {
        pluginId: outcome.conflictPluginId,
      }),
    );
    return;
  }
  if (outcome.state !== "pack_installed") {
    if (successDescription === undefined) {
      toast.success(t(successKey));
      return;
    }
    toast.success(t(successKey), { description: successDescription });
    return;
  }

  const description = packInstallDescription(outcome, t);
  if (outcome.failed !== null) {
    toast.error(t("settings.plugins.packInstallFailedTitle"), { description });
    return;
  }
  toast.success(t("settings.plugins.packInstallTitle"), { description });
}

/** Builds the journal-backed member summary shared by card and detail installs. */
function packInstallDescription(
  outcome: Extract<InstallOutcome, { state: "pack_installed" }>,
  t: TFunction,
): string {
  const parts: string[] = [];
  if (outcome.members.length > 0) {
    parts.push(
      t("settings.plugins.packInstalledMembers", {
        count: outcome.members.length,
      }),
    );
  }
  if (outcome.skipped.length > 0) {
    parts.push(
      t("settings.plugins.packSkippedMembers", {
        count: outcome.skipped.length,
      }),
    );
  }
  if (outcome.failed !== null) {
    parts.push(
      t("settings.plugins.packFailedMember", {
        pluginId: outcome.failed.pluginId,
      }),
    );
    for (const rollbackFailure of outcome.failed.rollbackFailures) {
      parts.push(
        t("settings.plugins.packRollbackFailedMember", {
          pluginId: rollbackFailure.pluginId,
        }),
      );
    }
  }
  return parts.join(" ");
}
