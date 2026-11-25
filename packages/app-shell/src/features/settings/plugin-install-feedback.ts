import type {
  InstallOutcome,
  MarketplaceWorkflowOutcome,
} from "@ora/contracts";
import { toast } from "@ora/ui";
import type { TFunction } from "i18next";

type InstallSuccessKey =
  "settings.plugins.installSuccess" | "settings.plugins.importSuccess";

/**
 * Which marketplace operation produced the document outcomes.
 *
 * The operation changes what the same outcome means: an install or a card retry is the user asking
 * for the documents now, so a name conflict is news and opens the report, while an automatic
 * upgrade only has to speak up for a conflict it has not reported before.
 */
export type MarketplaceImportTrigger = "install" | "update" | "retry";

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
 * are two facts about one operation; the details stay one click away in the workflow library. When
 * nothing was delivered the toast stops claiming a plain success, so the headline never contradicts
 * the documents the user is missing.
 */
export function showPluginInstallOutcome(
  outcome: InstallOutcome,
  t: TFunction,
  successKey: InstallSuccessKey = "settings.plugins.installSuccess",
  workflows: readonly MarketplaceWorkflowOutcome[] = [],
  trigger: MarketplaceImportTrigger = "install",
): void {
  const documentSummary = workflowDocumentSummary(workflows, t, trigger);
  const undelivered = documentsUndelivered(workflows, trigger);
  if (outcome.state !== "pack_installed") {
    // A package that contributed no documents keeps the plain one-argument toast its callers
    // already match against; only a document report turns it into a described toast.
    if (documentSummary === "") {
      toast.success(t(successKey));
      return;
    }
    if (undelivered) {
      toast.warning(t("settings.plugins.installSuccessDocumentsNotImported"), {
        description: documentSummary,
      });
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
  if (undelivered) {
    toast.warning(t("settings.plugins.packInstallDocumentsNotImported"), {
      description,
    });
    return;
  }
  toast.success(t("settings.plugins.packInstallTitle"), { description });
}

/**
 * Reports whether a document report means the operation delivered nothing usable.
 *
 * The rule is deliberately narrow: a document that arrived is worth a success headline even
 * beside failures, and an upgrade never turns a known conflict or a copy the user deleted into a
 * non-success style, because neither is news the upgrade created.
 */
export function documentsUndelivered(
  workflows: readonly MarketplaceWorkflowOutcome[],
  trigger: MarketplaceImportTrigger,
): boolean {
  if (workflows.some((outcome) => outcome.state === "imported")) return false;
  if (workflows.some((outcome) => outcome.state === "failed")) return true;
  const conflicts = workflows.filter(
    (outcome) => outcome.state === "skipped_conflict",
  );
  return trigger === "update"
    ? conflicts.some((outcome) => !outcome.knownConflict)
    : conflicts.length > 0;
}

/**
 * Summarises a marketplace document report as one line of counts.
 *
 * The states are reported separately because they mean different things to the user: imported
 * documents are new workflows, processed paths still have their copy, a deleted copy is the user's
 * own earlier choice with a recovery path, conflict skips name a workflow the user already had, and
 * failures are the part worth retrying. An upgrade states its known conflicts and deleted copies as
 * neutral counts instead of reopening the report, so a repeated upgrade does not look like a new
 * failure every time. An empty report produces no line at all, so an ordinary plugin install keeps
 * its plain toast.
 */
export function workflowDocumentSummary(
  workflows: readonly MarketplaceWorkflowOutcome[],
  t: TFunction,
  trigger: MarketplaceImportTrigger,
): string {
  let imported = 0;
  let processed = 0;
  let deleted = 0;
  let conflicts = 0;
  let knownConflicts = 0;
  let failed = 0;
  for (const outcome of workflows) {
    switch (outcome.state) {
      case "imported":
        imported += 1;
        break;
      case "skipped_processed":
        processed += 1;
        break;
      case "skipped_deleted_copy":
        deleted += 1;
        break;
      case "skipped_conflict":
        // A known conflict is only neutral while an upgrade reports it; an install or a retry is
        // asking for the documents now, so the same conflict is the actionable count there.
        if (outcome.knownConflict && trigger === "update") {
          knownConflicts += 1;
        } else {
          conflicts += 1;
        }
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
  if (deleted > 0) {
    parts.push(
      t("settings.plugins.workflowDocumentsDeleted", { count: deleted }),
    );
  }
  if (conflicts > 0) {
    parts.push(
      t("settings.plugins.workflowDocumentsConflicts", { count: conflicts }),
    );
  }
  if (knownConflicts > 0) {
    parts.push(
      t("settings.plugins.workflowDocumentsKnownConflicts", {
        count: knownConflicts,
      }),
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
