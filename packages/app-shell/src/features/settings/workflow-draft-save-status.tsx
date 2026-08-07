import { useTranslation } from "react-i18next";
import { cn } from "@ora/ui";
import type { WorkflowDraftSaveStatus } from "./use-workflow-draft-autosave";

type WorkflowDraftSaveStatusProps = {
  status: WorkflowDraftSaveStatus;
  /** Formatted draft `updated_at` after the last successful persist. */
  draftUpdatedAt?: string;
  className?: string;
};

/**
 * Surfaces autosave progress next to the workflow title without introducing a
 * manual Save control. Keeps the last live-saved timestamp visible while edits
 * are still inside the debounce window so the header does not flicker to
 * "unsaved" on every keystroke.
 */
export function WorkflowDraftSaveStatusLabel({
  status,
  draftUpdatedAt,
  className,
}: WorkflowDraftSaveStatusProps) {
  const { t } = useTranslation();

  let label: string;
  if (status === "saving") {
    label = t("settings.workflow.saving");
  } else if (status === "error") {
    label = t("settings.workflow.saveError");
  } else if (draftUpdatedAt !== undefined) {
    label = t("settings.workflow.liveSaved", { time: draftUpdatedAt });
  } else if (status === "dirty") {
    label = t("settings.workflow.unsaved");
  } else {
    label = t("settings.workflow.saved");
  }

  return (
    <p
      aria-live="polite"
      className={cn(
        "max-w-56 shrink-0 truncate text-right text-[10px] leading-4 text-muted-foreground",
        status === "error" && "text-destructive",
        className,
      )}
      title={label}
    >
      {label}
    </p>
  );
}
