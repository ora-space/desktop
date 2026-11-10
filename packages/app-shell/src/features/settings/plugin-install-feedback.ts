import type {
  InstallOutcome,
  MarketplaceWorkflowOutcome,
} from "@ora/contracts";
import { toast } from "@ora/ui";
import type { TFunction } from "i18next";

type InstallSuccessKey =
  "settings.plugins.installSuccess" | "settings.plugins.importSuccess";

/**
 * Presents one install outcome consistently across every marketplace entry point.
 *
 * `successKey` selects the wording of the plain success toast, so a local import can say the
 * package arrived without repeating the marketplace install message. Outcomes that already own
 * their wording (a command conflict, a pack member journal) ignore it, so a caller never
 * overrides the message its own outcome requires.
 *
 * `workflows` carries the per-document results a Workflow package reported. They are summarised in
 * the same toast instead of a second one, because a document outcome and the installation outcome
 * are two facts about one operation; the details stay one click away in the workflow library.
 */
export function showPluginInstallOutcome(
  outcome: InstallOutcome,
  t: TFunction,
  successKey: InstallSuccessKey = "settings.plugins.installSuccess",
  workflows: readonly MarketplaceWorkflowOutcome[] = [],
): void {
  const documentSummary = workflowDocumentSummary(workflows, t);
  if (outcome.state !== "pack_installed") {
    // A package that contributed no documents keeps the plain one-argument toast its callers
    // already match against; only a document report turns it into a described toast.
    if (documentSummary === "") {
      toast.success(t(successKey));
      return;
    }
    toast.success(t(successKey), { description: documentSummary });
    return;
  }

  const parts = [packInstallDescription(outcome, t), documentSummary].filter(
    (part) => part !== "",
  );
  const description = parts.join(" ");
  if (outcome.failed !== null) {
    toast.error(t("settings.plugins.packInstallFailedTitle"), { description });
    return;
  }
  toast.success(t("settings.plugins.packInstallTitle"), { description });
}

/**
 * Summarises a marketplace document report as one line of counts.
 *
 * The four states are reported separately because they mean different things to the user: imported
 * documents are new workflows, processed paths were handled by an earlier version, conflict skips
 * name a workflow the user already had, and failures are the only part still worth retrying. An
 * empty report produces no line at all, so an ordinary plugin install keeps its plain toast.
 */
export function workflowDocumentSummary(
  workflows: readonly MarketplaceWorkflowOutcome[],
  t: TFunction,
): string {
  let imported = 0;
  let processed = 0;
  let conflicts = 0;
  let failed = 0;
  for (const outcome of workflows) {
    switch (outcome.state) {
      case "imported":
        imported += 1;
        break;
      case "skipped_processed":
        processed += 1;
        break;
      case "skipped_conflict":
        conflicts += 1;
        break;
      case "failed":
        failed += 1;
        break;
    }
  }
  const parts: string[] = [];
  if (imported > 0) {
    parts.push(
      t("settings.plugins.workflowDocumentsImported", { count: imported }),
    );
  }
  if (processed > 0) {
    parts.push(
      t("settings.plugins.workflowDocumentsProcessed", { count: processed }),
    );
  }
  if (conflicts > 0) {
    parts.push(
      t("settings.plugins.workflowDocumentsConflicts", { count: conflicts }),
    );
  }
  if (failed > 0) {
    parts.push(
      t("settings.plugins.workflowDocumentsFailed", { count: failed }),
    );
  }
  return parts.join(" ");
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
