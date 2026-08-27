import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Button,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Input,
} from "@ora/ui";
import {
  IconChevronRight,
  IconGitBranch,
  IconMessageCircle,
  IconPlus,
  IconRoute,
  IconSearch,
} from "@tabler/icons-react";
import { useWorkflowLibrary } from "../workflow-editor/workflow-definitions";
import { useUiStore } from "../../state/stores/ui-store";

interface SidebarCreateMenuProps {
  /** Project used to group the created draft or workflow run in the sidebar. */
  projectId: string;
  /** Exact Workspace that owns chats and workflow execution created from this row. */
  workspaceId: string | null;
  scope: "project" | "task";
  onNewTask: () => void;
}

const ITEM_CLASS =
  "flex w-full cursor-default items-center gap-1.5 rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-accent focus-visible:bg-accent";

/**
 * Compact plus on a Workspace row: ordinary task, optional worktree task, or workflow run.
 *
 * The first panel is a Popover. Workflow templates open as a second floating
 * panel to the right and list only workflows that already have a published
 * snapshot — drafts are not runnable from here.
 */
export function SidebarCreateMenu({
  projectId,
  workspaceId,
  scope,
  onNewTask,
}: SidebarCreateMenuProps) {
  const { t } = useTranslation();
  const setDialog = useUiStore((s) => s.setDialog);
  const libraryQuery = useWorkflowLibrary();
  // Runs bind to the published snapshot. Drafts are authoring-only and would
  // fail create, so this picker never offers them.
  const workflows = useMemo(
    () =>
      (libraryQuery.data ?? []).filter(
        (workflow) => workflow.publishedVersion != null,
      ),
    [libraryQuery.data],
  );
  const [menuOpen, setMenuOpen] = useState(false);
  const [workflowOpen, setWorkflowOpen] = useState(false);
  const [templateQuery, setTemplateQuery] = useState("");
  const workflowSearchRef = useRef<HTMLInputElement>(null);
  const focusWorkflowSearch = useRef(false);
  const needle = templateQuery.trim().toLowerCase();
  const visibleWorkflows = useMemo(
    () =>
      needle
        ? workflows.filter((workflow) =>
            workflow.name.toLowerCase().includes(needle),
          )
        : workflows,
    [needle, workflows],
  );

  /** Closes both panels and clears the template search. */
  function closeAll(): void {
    setMenuOpen(false);
    setWorkflowOpen(false);
    setTemplateQuery("");
    focusWorkflowSearch.current = false;
  }

  useEffect(() => {
    if (!workflowOpen || !focusWorkflowSearch.current) return;
    // Keyboard open requested focus. Hover must not steal it from the trigger.
    const frame = window.requestAnimationFrame(() => {
      workflowSearchRef.current?.focus();
      focusWorkflowSearch.current = false;
    });
    return () => window.cancelAnimationFrame(frame);
  }, [workflowOpen]);

  return (
    <Popover
      open={menuOpen}
      onOpenChange={(open) => {
        setMenuOpen(open);
        if (!open) {
          setWorkflowOpen(false);
          setTemplateQuery("");
          focusWorkflowSearch.current = false;
        }
      }}
    >
      <PopoverTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={t(
              scope === "project"
                ? "sidebar.createInProject"
                : "sidebar.createInTask",
            )}
            title={t(
              scope === "project"
                ? "sidebar.createInProject"
                : "sidebar.createInTask",
            )}
            onClick={(event) => {
              // The project row toggles expansion; opening create must not.
              event.stopPropagation();
            }}
          />
        }
      >
        <IconPlus />
      </PopoverTrigger>
      <PopoverContent
        align="end"
        side="bottom"
        sideOffset={6}
        className="w-max min-w-max gap-0.5 p-1"
      >
        <button
          type="button"
          className={ITEM_CLASS}
          onMouseEnter={() => setWorkflowOpen(false)}
          onClick={() => {
            closeAll();
            onNewTask();
          }}
        >
          <IconMessageCircle className="size-4" />
          {t("sidebar.newDirectChat")}
        </button>
        {scope === "project" && (
          <button
            type="button"
            className={ITEM_CLASS}
            onMouseEnter={() => setWorkflowOpen(false)}
            onClick={() => {
              closeAll();
              setDialog({ kind: "task", projectId });
            }}
          >
            <IconGitBranch className="size-4" />
            {t("sidebar.newTask")}
          </button>
        )}
        <Popover
          open={workflowOpen}
          onOpenChange={(open, details) => {
            // Hover already opened the panel. The click that follows would
            // otherwise toggle it shut via the trigger or an outside-press
            // on that same button (the popup is portaled beside it).
            if (
              !open &&
              (details.reason === "trigger-press" ||
                details.reason === "outside-press")
            ) {
              return;
            }
            setWorkflowOpen(open);
          }}
        >
          <PopoverTrigger
            render={
              <button
                type="button"
                className={ITEM_CLASS}
                disabled={workspaceId === null}
                aria-haspopup="true"
                aria-expanded={workflowOpen}
                onMouseEnter={() => setWorkflowOpen(true)}
                onPointerDown={(event) => {
                  // Hover already opened the panel; the click that follows
                  // must not toggle it shut through Popover's trigger.
                  event.preventDefault();
                  setWorkflowOpen(true);
                }}
                onKeyDown={(event) => {
                  if (event.key !== "ArrowRight" && event.key !== "Enter") {
                    return;
                  }
                  event.preventDefault();
                  if (workflowOpen) {
                    workflowSearchRef.current?.focus();
                    return;
                  }
                  focusWorkflowSearch.current = true;
                  setWorkflowOpen(true);
                }}
              />
            }
          >
            <IconRoute className="size-4" />
            {t("sidebar.newWorkflow")}
            <IconChevronRight className="ml-auto size-3.5 opacity-50" />
          </PopoverTrigger>
          <PopoverContent
            align="start"
            side="right"
            alignOffset={-4}
            sideOffset={6}
            className="w-44 gap-1 p-1"
          >
            <div className="relative">
              <IconSearch className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
              <Input
                ref={workflowSearchRef}
                value={templateQuery}
                onChange={(event) => setTemplateQuery(event.target.value)}
                placeholder={t("sidebar.searchWorkflows")}
                aria-label={t("sidebar.searchWorkflows")}
                className="h-7 min-w-0 border-transparent bg-muted/50 pl-7 text-xs shadow-none"
                onKeyDown={(event) => event.stopPropagation()}
                onClick={(event) => event.stopPropagation()}
              />
            </div>
            <div className="scrollbar-hide max-h-52 overflow-y-auto">
              {libraryQuery.isPending && (
                <p className="px-2 py-2 text-xs text-muted-foreground">
                  {t("sidebar.loading")}
                </p>
              )}
              {!libraryQuery.isPending && visibleWorkflows.length === 0 && (
                <p className="px-2 py-2 text-xs text-muted-foreground">
                  {t("sidebar.noWorkflows")}
                </p>
              )}
              {visibleWorkflows.map((workflow) => (
                <button
                  key={workflow.id}
                  type="button"
                  title={workflow.name}
                  className={`${ITEM_CLASS} min-w-0`}
                  onClick={() => {
                    if (workspaceId === null) return;
                    closeAll();
                    setDialog({
                      kind: "runWorkflow",
                      projectId,
                      workspaceId,
                      workflowId: workflow.id,
                      workflowName: workflow.name,
                    });
                  }}
                >
                  <span className="min-w-0 flex-1 truncate">
                    {workflow.name}
                  </span>
                </button>
              ))}
            </div>
          </PopoverContent>
        </Popover>
      </PopoverContent>
    </Popover>
  );
}
