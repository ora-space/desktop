import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import {
  Button,
  Collapsible,
  CollapsibleContent,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Input,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@ora/ui";
import {
  IconChevronDown,
  IconChevronRight,
  IconAlertTriangle,
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
import { useUnreadSessionsStore } from "../../state/stores/unread-sessions-store";
import { OraMark } from "../../components/ora-mark";
import { AgentActivityDots } from "../../components/agent-activity-dots";
import { DragRegion } from "../../components/drag-region";
import { useChatStore } from "../../chat-store-context";

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
  const chatStore = useChatStore();
  const conversations = useStore(chatStore, (state) => state.conversations);
  const unread = useUnreadSessionsStore((s) => s.unread);
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
        || sessions.some((session) => session.taskId === task.id && session.agentCli.toLowerCase().includes(needle)));
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
        <header className="flex h-14 items-center gap-2 px-3">
          <DragRegion>
            <OraMark size="default" />
            <span className="text-[15px] font-semibold tracking-[-0.01em]">Ora</span>
          </DragRegion>
          <Tooltip>
            <TooltipTrigger render={<Button variant="ghost" size="icon" onClick={() => setSidebarCollapsed(true)} aria-label={t("sidebar.collapse")} />}>
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
            className="h-10 w-full justify-start gap-2.5 px-2.5 text-sm font-medium"
          >
            <IconSquareRoundedPlus className="size-5" />
            {t("chat.newThread")}
            <span className="ml-auto text-xs font-normal text-muted-foreground">⌘N</span>
          </Button>
        </div>

        <div className="flex items-center gap-2 px-2 pb-3">
          <div className="relative min-w-0 flex-1">
            <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("sidebar.search")}
              className="h-9 border-transparent bg-sidebar-accent/60 px-8 text-[13px] shadow-none hover:bg-sidebar-accent focus-visible:bg-background"
            />
            {query && (
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
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
          <div className="flex h-8 items-center px-2 text-xs font-medium text-muted-foreground">
            <span>{t("sidebar.projects")}</span>
            <Tooltip>
              <TooltipTrigger render={<Button variant="ghost" size="icon-sm" className="ml-auto" onClick={() => setDialog({ kind: "project" })} aria-label={t("sidebar.newProject")} />}>
                <IconPlus />
              </TooltipTrigger>
              <TooltipContent>{t("sidebar.newProject")}</TooltipContent>
            </Tooltip>
          </div>
          {loading && <p className="px-2 py-6 text-center text-[13px] text-muted-foreground">{t("sidebar.loading")}</p>}
          {!loading && visibleProjects.length === 0 && (
            <p className="px-2 py-6 text-center text-[13px] text-muted-foreground">{t("sidebar.empty")}</p>
          )}
          {visibleProjects.map((project) => {
            const projectTasks = tasks.filter((task) => task.projectId === project.id);
            const projectOpen = expandedProjects.has(project.id) || Boolean(needle);
            return (
              <div key={project.id}>
                <TreeRow
                  depth={0}
                  active={selection.projectId === project.id && selection.taskId === null}
                  icon={<IconFolder className="size-[18px] text-muted-foreground" />}
                  label={project.name}
                  expanded={projectOpen}
                  onClick={() => openProject(project.id)}
                  action={<NewTaskButton onClick={() => setDialog({ kind: "task", projectId: project.id })} />}
                  menu={(
                    <EntityMenu
                      onEdit={() => setDialog({ kind: "project", entity: project })}
                      onDelete={() => setDeleteTarget({ kind: "project", id: project.id, name: project.name })}
                    />
                  )}
                />
                <TreeBranch expanded={projectOpen}>
                  {projectTasks.map((task) => {
                    const taskSessions = sessions.filter((session) => session.taskId === task.id);
                    const taskOpen = expandedTasks.has(task.id) || Boolean(needle);
                    return (
                      <div key={task.id}>
                        <TreeRow
                          depth={1}
                          active={selection.taskId === task.id && selection.sessionId === null}
                          icon={<IconGitBranch className="size-4 text-muted-foreground" />}
                          label={task.title}
                          expanded={taskOpen}
                          onClick={() => openTask(task.id)}
                          action={<NewSessionButton onClick={() => selectTask(task.id, task.projectId)} />}
                          menu={(
                            <EntityMenu
                              onEdit={() => setDialog({ kind: "task", projectId: project.id, entity: task })}
                              onDelete={() => setDeleteTarget({ kind: "task", id: task.id, name: task.title })}
                            />
                          )}
                        />
                        <TreeBranch expanded={taskOpen}>
                          {taskSessions.map((session) => (
                            <TreeRow
                              key={session.id}
                              depth={2}
                              active={selection.sessionId === session.id}
                              // The dots mean "the agent is working right now", which is the
                              // live prompt activity in the chat store - not session.status,
                              // which tracks whether the backing process is alive and so stays
                              // "running" through every idle gap between turns. Once it stops,
                              // the same slot carries an unread mark until the session is opened.
                              icon={conversations[session.id]?.pendingPermissions.length
                                ? <IconAlertTriangle className="size-[18px] text-amber-500" aria-label={t("sidebar.permissionRequired")} />
                                : conversations[session.id]?.isResponding
                                  ? <AgentActivityDots label={t("common.running")} className="text-muted-foreground" />
                                  : unread.has(session.id)
                                    ? <UnreadDot label={t("sidebar.unread")} />
                                    : null}
                              label={session.agentCli}
                              onClick={() => selectSession(session.id, task.id, project.id)}
                              menu={(
                                <EntityMenu
                                  onDelete={() => setDeleteTarget({ kind: "session", id: session.id, name: session.agentCli })}
                                />
                              )}
                            />
                          ))}
                        </TreeBranch>
                      </div>
                    );
                  })}
                </TreeBranch>
              </div>
            );
          })}
        </nav>

        {error && <p data-selectable className="border-t border-destructive/20 bg-destructive/10 px-3 py-2 text-xs text-destructive">{error.message}</p>}
        <div className="p-2">
          <UserProfile user={user} onOpenSettings={() => setSettingsOpen(true)} onSignOut={onSignOut} />
        </div>
      </aside>
    </>
  );
}

/**
 * Marks a session that finished a turn the user has not opened yet.
 *
 * A single filled blue dot - `sky`, the same blue the chat surface already uses
 * for tool-call chrome - so it reads as "new here" and stays clear of the status
 * colours (emerald means running, amber means a permission is waiting). It sits
 * in the icon slot the activity dots vacate when the turn ends, so a row never
 * shows both at once.
 */
function UnreadDot({ label }: { label: string }) {
  return (
    <span
      role="img"
      aria-label={label}
      className="size-2 rounded-full bg-sky-500"
    />
  );
}

/**
 * Animates a level of the tree open and closed.
 *
 * Driven by the shared Collapsible rather than a hand-rolled height, because the
 * same sidebar ships to the desktop shell and the browser: both put it on WebKit,
 * where animating a `0fr`/`1fr` grid track is far less dependable than the pixel
 * height Base UI measures into `--collapsible-panel-height`.
 *
 * The rows carry their own selection state, so the row button stays the control
 * and this stays a controlled panel with no Trigger of its own.
 *
 * Follows the height pattern established by the shared Accordion. Note that
 * tw-animate-css's `animate-collapsible-*` classes cannot stand in here: their
 * keyframes read Radix/Bits/Reka/Kobalte height variables, none of which Base UI
 * sets, so they would silently fall back to `height: auto` and never animate.
 */
function TreeBranch({ expanded, children }: { expanded: boolean; children: React.ReactNode }) {
  return (
    <Collapsible open={expanded}>
      <CollapsibleContent className="h-(--collapsible-panel-height) overflow-hidden transition-[height,opacity] duration-200 ease-out data-ending-style:h-0 data-ending-style:opacity-0 data-starting-style:h-0 data-starting-style:opacity-0">
        {children}
      </CollapsibleContent>
    </Collapsible>
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
  /** Optional primary command shown beside the overflow menu on hover. */
  action?: React.ReactNode;
  menu: React.ReactNode;
}

/** Keeps every tree level aligned while preserving a stable row width for actions. */
function TreeRow({ depth, active, icon, label, meta, expanded, onClick, action, menu }: TreeRowProps) {
  return (
    <div className={`group/tree flex h-9 items-center rounded-md transition-colors ${active ? "bg-sidebar-accent text-sidebar-accent-foreground" : "hover:bg-sidebar-accent/70"}`}>
      <button
        type="button"
        onClick={onClick}
        aria-expanded={expanded}
        className="flex h-full min-w-0 flex-1 items-center gap-2 rounded-md text-left text-[13px] outline-none focus-visible:ring-2 focus-visible:ring-ring"
        style={{ paddingLeft: `${8 + depth * 18}px` }}
      >
        <span className="relative flex size-[18px] shrink-0 items-center justify-center">
          <span className={`flex items-center justify-center transition-opacity duration-100 ${expanded === undefined ? "" : "group-hover/tree:opacity-0"}`}>{icon}</span>
          {expanded !== undefined && (expanded
            ? <IconChevronDown className="absolute size-4 opacity-0 transition-opacity duration-100 group-hover/tree:opacity-100" />
            : <IconChevronRight className="absolute size-4 opacity-0 transition-opacity duration-100 group-hover/tree:opacity-100" />)}
        </span>
        <span className="min-w-0 flex-1 truncate font-medium">{label}</span>
        {meta && <span className="truncate text-[11px] text-muted-foreground">{meta}</span>}
      </button>
      <div className="mr-1 flex items-center opacity-0 transition-opacity duration-100 group-hover/tree:opacity-100 group-focus-within/tree:opacity-100">
        {menu}
        {action}
      </div>
    </div>
  );
}

/**
 * Opens the composer for a new session against the row's own scope.
 *
 * Selecting the row's entity is the whole implementation: the workspace shows the
 * composer for any selection without a session, and the context bar reads the
 * same selection, so a project row lands on that project and a worktree row lands
 * on that project plus branch.
 */
function NewSessionButton({ onClick }: { onClick: () => void }) {
  const { t } = useTranslation();
  return (
    <Button
      variant="ghost"
      size="icon-sm"
      aria-label={t("sidebar.newSession")}
      onClick={(event) => {
        // The row underneath toggles expansion; opening the composer should not.
        event.stopPropagation();
        onClick();
      }}
    >
      <IconPlus />
    </Button>
  );
}

/**
 * Opens the create-worktree-task dialog scoped to the row's project.
 *
 * A project's primary "+" creates a new worktree task rather than dropping
 * straight into a composer: a task owns the branch a session runs against, so
 * this is the entry point that gives later sessions somewhere to attach.
 */
function NewTaskButton({ onClick }: { onClick: () => void }) {
  const { t } = useTranslation();
  return (
    <Button
      variant="ghost"
      size="icon-sm"
      aria-label={t("sidebar.newTask")}
      onClick={(event) => {
        // The row underneath toggles expansion; opening the dialog should not.
        event.stopPropagation();
        onClick();
      }}
    >
      <IconPlus />
    </Button>
  );
}

/** Provides contextual CRUD commands without making every tree row visually noisy. */
function EntityMenu({ onEdit, onDelete }: { onEdit?: () => void; onDelete: () => void }) {
  const { t } = useTranslation();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" aria-label={t("sidebar.openActions")} onClick={(event) => event.stopPropagation()} />}>
        <IconDots />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-44">
        {onEdit && <DropdownMenuItem onClick={onEdit}><IconPencil />{t("common.edit")}</DropdownMenuItem>}
        <DropdownMenuItem variant="destructive" onClick={onDelete}><IconTrash />{t("common.delete")}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
