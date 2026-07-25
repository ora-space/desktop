import { useTranslation } from "react-i18next";
import { IconRoute } from "@tabler/icons-react";
import { Button } from "@ora/ui";
import { getRun, useWorkflowStore } from "../../state/stores/workflow-store";
import { useWorkflowKey } from "./use-workflow-key";

/**
 * Composer toggle that shows or hides the current session's spec-driven stepper.
 * The first press starts a workflow (and arms the explore step); later presses
 * only flip visibility, so the progress persists until the stepper's Cancel
 * button resets it.
 */
export function WorkflowToggle({ disabled = false }: { disabled?: boolean }) {
  const { t } = useTranslation();
  const key = useWorkflowKey();
  const visible = useWorkflowStore((state) => getRun(state, key).visible);
  const toggleVisible = useWorkflowStore((state) => state.toggleVisible);

  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      disabled={disabled}
      aria-pressed={visible}
      aria-label={t("workflow.toggle")}
      onClick={() => toggleVisible(key)}
      className={
        visible
          ? "h-7 gap-1.5 rounded-md px-2 text-xs font-medium text-sky-600 ring-1 ring-inset ring-sky-500/30 hover:bg-sky-500/10 hover:text-sky-600"
          : "h-7 gap-1.5 rounded-md px-2 text-xs font-normal text-muted-foreground hover:bg-muted/60 hover:text-foreground"
      }
    >
      <IconRoute className="size-3.5 shrink-0" />
      <span className="whitespace-nowrap">{t("workflow.toggle")}</span>
    </Button>
  );
}
