import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Input,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@ora/ui";
import {
  IconChevronDown,
  IconChevronRight,
  IconDots,
  IconFolder,
  IconGitBranch,
  IconLayoutSidebarLeftCollapse,
  IconPencil,
  IconPlus,
  IconSearch,
  IconSquareRoundedPlus,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import type { CurrentUser } from "../../lib/types";
import { UserProfile } from "../sidebar/user-profile";
import { useProjects } from "../../state/hooks/use-projects";
import { useTasks } from "../../state/hooks/use-tasks";
import { useSessions } from "../../state/hooks/use-sessions";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { OraMark } from "../../components/ora-mark";

interface WorkspaceSidebarProps {
  user: CurrentUser;
  onSignOut: () => void;
}

/** Renders projects, worktree tasks, and agent sessions as a dense three-level navigation tree. */
export function WorkspaceSidebar({ user, onSignOut }: WorkspaceSidebarProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const initializedTreeExpansion = useRef(false);

  const projectsQuery = useProjects();
  const tasksQuery = useTasks();
  const sessionsQuery = useSessions();
  // Stabilise the array references so useMemo dependencies don't change every render.
  const projects = useMemo(() => projectsQuery.data ?? [], [projectsQuery.data]);
  const tasks = useMemo(() => tasksQuery.data ?? [], [tasksQuery.data]);
  const sessions = useMemo(() => sessionsQuery.data ?? [], [sessionsQuery.data]);
  const loading = projectsQuery.isPending || tasksQuery.isPending || sessionsQuery.isPending;
  const error = projectsQuery.error ?? tasksQuery.error ?? sessionsQuery.error;

  const selection = useWorkspaceSelectionStore((s) => s.selection);
  const selectProject = useWorkspaceSelectionStore((s) => s.selectProject);
  const selectTask = useWorkspaceSelectionStore((s) => s.selectTask);
  const selectSession = useWorkspaceSelectionStore((s) => s.selectSession);
  const clearSelection = useWorkspaceSelectionStore((s) => s.clearSelection);

  const expandedProjects = useUiStore((s) => s.expandedProjects);
  const expandedTasks = useUiStore((s) => s.expandedTasks);
  const toggleProjectExpand = useUiStore((s) => s.toggleProjectExpand);
  const toggleTaskExpand = useUiStore((s) => s.toggleTaskExpand);
  const setSidebarCollapsed = useUiStore((s) => s.setSidebarCollapsed);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const setDialog = useUiStore((s) => s.setDialog);
  const setDeleteTarget = useUiStore((s) => s.setDeleteTarget);

  const needle = query.trim().toLowerCase();

  const visibleProjects = useMemo(() => projects.filter((project) => {
    if (!needle) return true;
    const projectTasks = tasks.filter((task) => task.projectId === project.id);
    return project.name.toLowerCase().includes(needle)
      || projectTasks.some((task) => task.title.toLowerCase().includes(needle)
        || sessions.some((session) => session.taskId === task.id && session.agentId.toLowerCase().includes(needle)));
  }), [needle, projects, sessions, tasks]);

  // Expand the initial workspace tree once while preserving later manual collapse choices.
  useEffect(() => {
    if (loading || initializedTreeExpansion.current) return;
    initializedTreeExpansion.current = true;
    useUiStore.setState((state) => ({
      expandedProjects: new Set([...state.expandedProjects, ...projects.map((project) => project.id)]),
      expandedTasks: new Set([...state.expandedTasks, ...tasks.map((task) => task.id)]),
    }));
  }, [loading, projects, tasks]);

  // Mutations select their new child. Expand its ancestors once without preventing a later manual collapse.
  useEffect(() => {
    if (selection.taskId) useUiStore.getState().expandTask(selection.taskId);
    if (selection.projectId) useUiStore.getState().expandProject(selection.projectId);
  }, [selection.projectId, selection.taskId]);

  const openProject = (projectId: string) => {
    toggleProjectExpand(projectId);
    selectProject(projectId);
  };

  const openTask = (taskId: string) => {
    const task = tasks.find((candidate) => candidate.id === taskId);
    if (task) {
      toggleTaskExpand(taskId);
      selectTask(taskId, task.projectId);
    }
  };

  // Conversations are keyed by Ora session, so "new chat" is just dropping the
  // current selection: the workspace falls back to the empty composer.
  const openNewChat = () => {
    clearSelection();
  };

  // Match desktop IDE conventions while preventing the browser's new-window shortcut.
  useEffect(() => {
    const handleNewChatShortcut = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "n") {
        event.preventDefault();
        clearSelection();
      }
    };
    window.addEventListener("keydown", handleNewChatShortcut);
    return () => window.removeEventListener("keydown", handleNewChatShortcut);
  }, [clearSelection]);

  return (
    <>
      {/* Width is owned by the enclosing ResizablePanel, so the aside just fills it. */}
      <aside className="flex size-full min-w-0 flex-col bg-sidebar text-sidebar-foreground">
        <header className="flex h-13 items-center gap-2 px-3">
          <OraMark size="sm" />
          <span className="text-[13px] font-semibold tracking-[-0.01em]">Ora</span>
          <div className="flex-1" />
          <Tooltip>
            <TooltipTrigger render={<Button variant="ghost" size="icon-sm" onClick={() => setSidebarCollapsed(true)} aria-label={t("sidebar.collapse")} />}>
              <IconLayoutSidebarLeftCollapse />
            </TooltipTrigger>
            <TooltipContent>{t("sidebar.collapse")}</TooltipContent>
          </Tooltip>
        </header>

        <div className="px-2 pb-2">
          <Button
            type="button"
            variant="ghost"
            onClick={openNewChat}
            className="h-9 w-full justify-start gap-2.5 px-2.5 text-[13px] font-medium"
          >
            <IconSquareRoundedPlus className="size-[18px]" />
            {t("chat.newThread")}
            <span className="ml-auto text-[11px] font-normal text-muted-foreground">⌘N</span>
          </Button>
        </div>

        <div className="flex items-center gap-2 px-2 pb-3">
          <div className="relative min-w-0 flex-1">
            <IconSearch className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("sidebar.search")}
              className="h-8 border-transparent bg-sidebar-accent/60 px-7 text-xs shadow-none hover:bg-sidebar-accent focus-visible:bg-background"
            />
            {query && (
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                className="absolute right-1 top-1/2 -translate-y-1/2"
                aria-label={t("sidebar.clearSearch")}
                onClick={() => setQuery("")}
              >
                <IconX />
              </Button>
            )}
          </div>
        </div>

        <nav className="min-h-0 flex-1 overflow-y-auto px-2 pb-3" aria-label={t("sidebar.navigation")}>
          <div className="flex h-7 items-center px-2 text-[11px] font-medium text-muted-foreground">
            <span>{t("sidebar.projects")}</span>
            <Tooltip>
              <TooltipTrigger render={<Button variant="ghost" size="icon-xs" className="ml-auto" onClick={() => setDialog({ kind: "project" })} aria-label={t("sidebar.newProject")} />}>
                <IconPlus />
              </TooltipTrigger>
              <TooltipContent>{t("sidebar.newProject")}</TooltipContent>
            </Tooltip>
          </div>
          {loading && <p className="px-2 py-6 text-center text-xs text-muted-foreground">{t("sidebar.loading")}</p>}
          {!loading && visibleProjects.length === 0 && (
            <p className="px-2 py-6 text-center text-xs text-muted-foreground">{t("sidebar.empty")}</p>
          )}
          {visibleProjects.map((project) => {
            const projectTasks = tasks.filter((task) => task.projectId === project.id);
            const projectOpen = expandedProjects.has(project.id) || Boolean(needle);
            return (
              <div key={project.id}>
                <TreeRow
                  depth={0}
                  active={selection.projectId === project.id && selection.taskId === null}
                  icon={<IconFolder className="size-4 text-muted-foreground" />}
                  label={project.name}
                  meta={`${projectTasks.length}`}
                  expanded={projectOpen}
                  onClick={() => openProject(project.id)}
                  menu={(
                    <EntityMenu
                      onAdd={() => setDialog({ kind: "task", projectId: project.id })}
                      addLabel={t("sidebar.newWorktree")}
                      onEdit={() => setDialog({ kind: "project", entity: project })}
                      onDelete={() => setDeleteTarget({ kind: "project", id: project.id, name: project.name })}
                    />
                  )}
                />
                {projectOpen && projectTasks.map((task) => {
                  const taskSessions = sessions.filter((session) => session.taskId === task.id);
                  const taskOpen = expandedTasks.has(task.id) || Boolean(needle);
                  return (
                    <div key={task.id}>
                      <TreeRow
                        depth={1}
                        active={selection.taskId === task.id && selection.sessionId === null}
                        icon={<IconGitBranch className="size-3.5 text-muted-foreground" />}
                        label={task.title}
                        meta={t(`common.${task.status}`)}
                        expanded={taskOpen}
                        onClick={() => openTask(task.id)}
                        menu={(
                          <EntityMenu
                            onAdd={() => setDialog({ kind: "session", taskId: task.id })}
                            addLabel={t("sidebar.newSession")}
                            onEdit={() => setDialog({ kind: "task", projectId: project.id, entity: task })}
                            onDelete={() => setDeleteTarget({ kind: "task", id: task.id, name: task.title })}
                          />
                        )}
                      />
                      {taskOpen && taskSessions.map((session) => (
                        <TreeRow
                          key={session.id}
                          depth={2}
                          active={selection.sessionId === session.id}
                          icon={session.status === "running" ? <AgentActivityDots label={t("common.running")} /> : null}
                          label={session.agentId}
                          onClick={() => selectSession(session.id, task.id, project.id)}
                          menu={(
                            <EntityMenu
                              onEdit={() => setDialog({ kind: "session", taskId: task.id, entity: session })}
                              onDelete={() => setDeleteTarget({ kind: "session", id: session.id, name: session.agentId })}
                            />
                          )}
                        />
                      ))}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </nav>

        {error && <p className="border-t border-destructive/20 bg-destructive/10 px-3 py-2 text-xs text-destructive">{error.message}</p>}
        <div className="p-2">
          <UserProfile user={user} onOpenSettings={() => setSettingsOpen(true)} onSignOut={onSignOut} />
        </div>
      </aside>
    </>
  );
}

/**
 * Row-position animation for each cell of the 3x3 grid, in row-major order.
 *
 * Spelled out as whole class names because Tailwind scans source text: a name
 * assembled at runtime would never make it into the generated stylesheet.
 */
const AGENT_DOT_ANIMATIONS = [
  "animate-dot-column-top", "animate-dot-column-top", "animate-dot-column-top",
  "animate-dot-column-middle", "animate-dot-column-middle", "animate-dot-column-middle",
  "animate-dot-column-bottom", "animate-dot-column-bottom", "animate-dot-column-bottom",
];

/**
 * Offset between columns, one third of the 1.2s `dot-column-*` cycle.
 *
 * Those keyframes hold at the top for this long plus 60ms, which is what makes
 * a column start falling a beat after the column to its right arrives. Retiming
 * the animation means moving this and the cycle duration together, or the
 * handoff breaks instead of just running at a different speed.
 */
const AGENT_DOT_COLUMN_DELAY_MS = 400;

/**
 * Marks a working agent with a 3x3 grid of squares.
 *
 * Every column runs the same two-dot window that climbs to the top, pauses,
 * and drops back down; columns are offset from each other so the three never
 * move in lockstep.
 *
 * The animation carries the "still running" meaning on its own, so a stopped
 * session renders nothing at all. `TreeRow` reserves the icon slot either way,
 * which keeps every label aligned as the status flips.
 */
function AgentActivityDots({ label }: { label: string }) {
  return (
    <span role="img" aria-label={label} className="grid grid-cols-3 gap-[2px] text-muted-foreground">
      {AGENT_DOT_ANIMATIONS.map((animation, index) => (
        <span
          key={index}
          className={`size-[2.5px] rounded-[0.5px] bg-current ${animation}`}
          style={{ animationDelay: `${(index % 3) * AGENT_DOT_COLUMN_DELAY_MS}ms` }}
        />
      ))}
    </span>
  );
}

interface TreeRowProps {
  depth: 0 | 1 | 2;
  active: boolean;
  icon: React.ReactNode;
  label: string;
  meta?: string;
  expanded?: boolean;
  onClick: () => void;
  menu: React.ReactNode;
}

/** Keeps every tree level aligned while preserving a stable row width for actions. */
function TreeRow({ depth, active, icon, label, meta, expanded, onClick, menu }: TreeRowProps) {
  return (
    <div className={`group/tree flex h-8 items-center rounded-md transition-colors ${active ? "bg-sidebar-accent text-sidebar-accent-foreground" : "hover:bg-sidebar-accent/70"}`}>
      <button
        type="button"
        onClick={onClick}
        aria-expanded={expanded}
        className="flex h-full min-w-0 flex-1 items-center gap-1.5 rounded-md text-left text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring"
        style={{ paddingLeft: `${6 + depth * 16}px` }}
      >
        <span className="relative flex size-4 shrink-0 items-center justify-center">
          <span className={`flex items-center justify-center transition-opacity duration-100 ${expanded === undefined ? "" : "group-hover/tree:opacity-0"}`}>{icon}</span>
          {expanded !== undefined && (expanded
            ? <IconChevronDown className="absolute size-3.5 opacity-0 transition-opacity duration-100 group-hover/tree:opacity-100" />
            : <IconChevronRight className="absolute size-3.5 opacity-0 transition-opacity duration-100 group-hover/tree:opacity-100" />)}
        </span>
        <span className="min-w-0 flex-1 truncate font-medium">{label}</span>
        {meta && <span className="truncate text-[10px] text-muted-foreground">{meta}</span>}
      </button>
      <div className="mr-1 opacity-0 transition-opacity duration-100 group-hover/tree:opacity-100 group-focus-within/tree:opacity-100">{menu}</div>
    </div>
  );
}

/** Provides contextual CRUD commands without making every tree row visually noisy. */
function EntityMenu({ onAdd, addLabel, onEdit, onDelete }: { onAdd?: () => void; addLabel?: string; onEdit: () => void; onDelete: () => void }) {
  const { t } = useTranslation();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={<Button variant="ghost" size="icon-xs" aria-label={t("sidebar.openActions")} onClick={(event) => event.stopPropagation()} />}>
        <IconDots />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-40">
        {onAdd && <DropdownMenuItem onClick={onAdd}><IconPlus />{addLabel}</DropdownMenuItem>}
        {onAdd && <DropdownMenuSeparator />}
        <DropdownMenuItem onClick={onEdit}><IconPencil />{t("common.edit")}</DropdownMenuItem>
        <DropdownMenuItem variant="destructive" onClick={onDelete}><IconTrash />{t("common.delete")}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

