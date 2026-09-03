import {
  Fragment,
  memo,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import {
  Button,
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
  Input,
  toast,
} from "@ora/ui";
import {
  IconArchive,
  IconChevronDown,
  IconChevronRight,
  IconPencil,
  IconRoute,
  IconTrash,
} from "@tabler/icons-react";
import type { GraphWorkflowRunStatus } from "@ora/workflow-runtime";
import {
  useRenameWorkflowRun,
  useWorkflowRunsByProject,
} from "../../state/hooks/use-workflow-runs";
import { useInlineTreeRename } from "./use-inline-tree-rename";

/**
 * Mounts branch children while open; after the user reopens once, keeps them
 * mounted (hidden) on collapse so session rows and workflow queries do not
 * remount and flash.
 *
 * The first open→collapse cycle still unmounts so first-run expand-all does
 * not pin every subtree in memory. Search-forced expansion passes
 * `retainWhenCollapsed={false}` so clearing the query drops matching branches.
 */
export function TreeBranch({
  expanded,
  children,
  retainWhenCollapsed = true,
}: {
  expanded: boolean;
  children: ReactNode;
  /** When false, collapse always unmounts (search-forced expansion). */
  retainWhenCollapsed?: boolean;
}) {
  const [sticky, setSticky] = useState(false);
  const wasExpandedRef = useRef(expanded);
  const hadOpenBeforeRef = useRef(false);

  useEffect(() => {
    if (!retainWhenCollapsed) {
      wasExpandedRef.current = expanded;
      if (expanded) hadOpenBeforeRef.current = true;
      return;
    }
    if (expanded) {
      if (!wasExpandedRef.current && hadOpenBeforeRef.current) {
        setSticky(true);
      }
      hadOpenBeforeRef.current = true;
    }
    wasExpandedRef.current = expanded;
  }, [expanded, retainWhenCollapsed]);

  if (!retainWhenCollapsed) {
    if (!expanded) return null;
    return <>{children}</>;
  }

  if (expanded) return <>{children}</>;
  if (!sticky) return null;
  return (
    <div hidden inert>
      {children}
    </div>
  );
}

interface TreeRowCommand {
  label: string;
  icon: ReactNode;
  variant?: "destructive";
  /** Renders a divider above this item, matching session-row delete placement. */
  separatorBefore?: boolean;
  onSelect: () => void;
}

interface TreeRowProps {
  depth: 0 | 1 | 2;
  active: boolean;
  /**
   * Soft highlight when a descendant leaf is selected but this branch is
   * collapsed — keeps the open chat discoverable without changing expand state
   * or the real selection.
   */
  containsSelection?: boolean;
  /**
   * Soft highlight for create-focus rows that are not the live selection — shows
   * where New chat will land without implying the composer already switched.
   */
  createFocused?: boolean;
  icon: ReactNode;
  label: string;
  meta?: string;
  expanded?: boolean;
  onClick: () => void;
  /** Double-click shortcut; branch rows create a new task in place. */
  onDoubleClick?: () => void;
  /** Hover-only plus (create under a project or a new session under a worktree). */
  action?: ReactNode;
  /** Persists the inline rename; same editor as session rows. */
  onRename: (name: string) => Promise<unknown>;
  /** Right-click actions after Rename; overflow `···` menus are intentionally not used. */
  commands: TreeRowCommand[];
}

/**
 * One navigation row: click selects/toggles, right-click opens commands.
 *
 * The context-menu trigger wraps only the label so the hover plus can host its
 * own dropdown without nesting menus. A nested native button inside Base UI's
 * default trigger would swallow `contextmenu` in the Tauri WebKit/WebView2 shells.
 */
export function TreeRow({
  depth,
  active,
  containsSelection = false,
  createFocused = false,
  icon,
  label,
  meta,
  expanded,
  onClick,
  onDoubleClick,
  action,
  onRename,
  commands,
}: TreeRowProps) {
  const { t } = useTranslation();
  const {
    renaming,
    draft,
    setDraft,
    inputRef,
    restoreMenuFocus,
    beginRename,
    onInputKeyDown,
    onInputBlur,
    maxLength,
  } = useInlineTreeRename({ value: label, onCommit: onRename });

  // True leaf selection and collapsed-ancestor hint share the same selected
  // chrome; create-focus stays softer so New-chat targeting never looks live.
  const selected = active || containsSelection;
  const rowTone = selected
    ? "bg-sidebar-accent text-sidebar-accent-foreground"
    : createFocused
      ? "bg-sidebar-accent/45 text-sidebar-foreground ring-1 ring-inset ring-sidebar-accent/60"
      : "hover:bg-sidebar-accent/70";

  return (
    <div
      className={`group/tree flex h-9 items-center rounded-md transition-colors ${rowTone}`}
      data-selection-hint={containsSelection && !active ? "true" : undefined}
      data-create-focus={createFocused && !selected ? "true" : undefined}
    >
      <ContextMenu>
        <ContextMenuTrigger
          render={
            <div
              className="flex h-full min-w-0 flex-1 items-center"
              onContextMenu={(event) => event.preventDefault()}
            />
          }
        >
          {renaming ? (
            <div
              className="flex h-full min-w-0 flex-1 items-center gap-2"
              style={{ paddingLeft: `${8 + depth * 18}px` }}
            >
              <span className="flex size-[18px] shrink-0 items-center justify-center">
                {icon}
              </span>
              <Input
                ref={inputRef}
                value={draft}
                maxLength={maxLength}
                aria-label={t("sidebar.rename")}
                className="h-7 flex-1 border-transparent bg-background px-1.5 text-[13px] shadow-none"
                onChange={(event) => setDraft(event.target.value)}
                onClick={(event) => event.stopPropagation()}
                onKeyDown={onInputKeyDown}
                onBlur={onInputBlur}
              />
            </div>
          ) : (
            <div
              role="button"
              tabIndex={0}
              onClick={onClick}
              onDoubleClick={onDoubleClick}
              onKeyDown={(event) => {
                if (event.key !== "Enter" && event.key !== " ") return;
                event.preventDefault();
                onClick();
              }}
              aria-expanded={expanded}
              className="flex h-full min-w-0 flex-1 cursor-pointer items-center gap-2 rounded-md text-left text-[13px] outline-none focus-visible:ring-2 focus-visible:ring-ring"
              style={{ paddingLeft: `${8 + depth * 18}px` }}
            >
              <span className="relative flex size-[18px] shrink-0 items-center justify-center">
                <span
                  className={`flex items-center justify-center transition-opacity duration-100 ${expanded === undefined ? "" : "group-hover/tree:opacity-0"}`}
                >
                  {icon}
                </span>
                {expanded !== undefined &&
                  (expanded ? (
                    <IconChevronDown className="absolute size-4 opacity-0 transition-opacity duration-100 group-hover/tree:opacity-100" />
                  ) : (
                    <IconChevronRight className="absolute size-4 opacity-0 transition-opacity duration-100 group-hover/tree:opacity-100" />
                  ))}
              </span>
              <span className="min-w-0 flex-1 truncate font-medium">
                {label}
              </span>
              {meta && (
                <span
                  className={`truncate text-[11px] ${active ? "text-sidebar-accent-foreground/80" : "text-amber-700 dark:text-amber-300"}`}
                >
                  {meta}
                </span>
              )}
            </div>
          )}
        </ContextMenuTrigger>
        {/* Rename suppresses restore so the editor keeps focus; other actions still return it. */}
        <ContextMenuContent
          className="w-44"
          finalFocus={() => restoreMenuFocus.current}
        >
          <ContextMenuItem onClick={beginRename}>
            <IconPencil />
            {t("sidebar.rename")}
          </ContextMenuItem>
          {commands.map((command) => (
            <Fragment key={command.label}>
              {command.separatorBefore ? <ContextMenuSeparator /> : null}
              <ContextMenuItem
                variant={command.variant}
                onClick={command.onSelect}
              >
                {command.icon}
                {command.label}
              </ContextMenuItem>
            </Fragment>
          ))}
        </ContextMenuContent>
      </ContextMenu>
      {action && !renaming && (
        <div className="mr-1 flex items-center opacity-0 transition-opacity duration-100 group-hover/tree:opacity-100 group-focus-within/tree:opacity-100">
          {action}
        </div>
      )}
    </div>
  );
}

/**
 * Placeholder archive control until persistence ships.
 * Same toast as session rows so every tree leaf exposes the same control.
 */
function ArchiveButton() {
  const { t } = useTranslation();
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-sm"
      aria-label={t("sidebar.archive")}
      onClick={(event) => {
        event.stopPropagation();
        toast(t("sidebar.archiveSoon"));
      }}
    >
      <IconArchive />
    </Button>
  );
}

/** Status dot color for sidebar GraphWorkflowRun rows. */
function runStatusClass(status: GraphWorkflowRunStatus): string {
  switch (status) {
    case "running":
      return "bg-sky-500";
    case "awaiting_input":
      return "bg-amber-500";
    case "succeeded":
      return "bg-emerald-500";
    case "failed":
      return "bg-rose-500";
    case "cancelled":
      return "bg-zinc-400";
    case "pending":
      return "bg-amber-400";
  }
}

/** Renders workflow runs belonging to one workspace within a project run query. */
export const ProjectWorkflowRunRows = memo(function ProjectWorkflowRunRows({
  projectId,
  workspaceId,
  depth,
  activeRunId,
  onSelectRun,
  onDeleteRun,
  listEnabled = true,
}: {
  projectId: string;
  workspaceId: string | null;
  depth: 1 | 2;
  activeRunId: string | null;
  onSelectRun: (runId: string) => void;
  onDeleteRun: (run: { id: string; name: string }) => void;
  /** False while the branch is collapsed so sticky keep-mount does not keep polling. */
  listEnabled?: boolean;
}) {
  const { t } = useTranslation();
  const runsQuery = useWorkflowRunsByProject(projectId, {
    enabled: listEnabled,
  });
  const renameWorkflowRun = useRenameWorkflowRun();
  const runs = (runsQuery.data ?? []).filter(
    (run) => run.workspaceId === workspaceId,
  );
  return (
    <>
      {runs.map((run) => {
        // The backend derives `awaitingInput` on the wire while the display model spells it
        // `awaiting_input`; normalize so the sidebar dot and label match the run detail.
        const displayStatus: GraphWorkflowRunStatus =
          run.status === "awaitingInput" ? "awaiting_input" : run.status;
        return (
          <TreeRow
            key={run.id}
            depth={depth}
            active={activeRunId === run.id}
            icon={
              <span className="relative flex size-[18px] items-center justify-center">
                <IconRoute
                  className="size-4 text-muted-foreground"
                  aria-hidden
                />
                <span
                  className={`absolute -right-0.5 -top-0.5 size-1.5 rounded-full ${runStatusClass(displayStatus)}`}
                  aria-label={t(`workflowRun.status.${displayStatus}`)}
                />
              </span>
            }
            label={run.name}
            onClick={() => onSelectRun(run.id)}
            action={<ArchiveButton />}
            onRename={(name) =>
              renameWorkflowRun.mutateAsync({
                runId: run.id,
                name,
                projectId,
              })
            }
            commands={[
              {
                label: t("sidebar.archive"),
                icon: <IconArchive />,
                onSelect: () => toast(t("sidebar.archiveSoon")),
              },
              {
                label: t("common.delete"),
                icon: <IconTrash />,
                variant: "destructive",
                separatorBefore: true,
                onSelect: () => onDeleteRun({ id: run.id, name: run.name }),
              },
            ]}
          />
        );
      })}
    </>
  );
});
