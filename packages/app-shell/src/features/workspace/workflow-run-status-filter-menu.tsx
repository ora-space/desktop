import { useTranslation } from "react-i18next";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@ora/ui";
import { IconFilter } from "@tabler/icons-react";
import {
  useUiStore,
  type SidebarWorkflowRunStatusFilter,
} from "../../state/stores/ui-store";
import {
  SIDEBAR_WORKFLOW_RUN_STATUS_FILTERS,
  sidebarRunStatusDotClass,
} from "./sidebar-workflow-run-filter";

/**
 * Per-project status filter for sidebar workflow rows.
 *
 * HITL uses the display spelling `awaiting_input` so the menu matches Theater
 * and the status dots rather than the wire `awaitingInput` token.
 */
export function WorkflowRunStatusFilterMenu({
  projectId,
}: {
  projectId: string;
}) {
  const { t } = useTranslation();
  const filter = useUiStore(
    (state) => state.workflowRunStatusFilterByProjectId[projectId] ?? "all",
  );
  const setFilter = useUiStore((state) => state.setWorkflowRunStatusFilter);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className={
              filter === "all" ? "text-muted-foreground" : "text-foreground"
            }
            aria-label={t("sidebar.filterWorkflowStatus")}
            aria-pressed={filter !== "all"}
          />
        }
      >
        <IconFilter className="size-3.5" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-44" side="bottom">
        <DropdownMenuRadioGroup
          value={filter}
          onValueChange={(value) => {
            if (
              !(
                SIDEBAR_WORKFLOW_RUN_STATUS_FILTERS as readonly string[]
              ).includes(value)
            ) {
              return;
            }
            setFilter(projectId, value as SidebarWorkflowRunStatusFilter);
          }}
        >
          {SIDEBAR_WORKFLOW_RUN_STATUS_FILTERS.map((value) => (
            <DropdownMenuRadioItem key={value} value={value} className="gap-2">
              {value === "all" ? null : (
                <span
                  className={`size-2 shrink-0 rounded-full ${sidebarRunStatusDotClass(value)}`}
                  aria-hidden
                />
              )}
              {value === "all"
                ? t("sidebar.filterWorkflowStatusAll")
                : t(`workflowRun.status.${value}`)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
