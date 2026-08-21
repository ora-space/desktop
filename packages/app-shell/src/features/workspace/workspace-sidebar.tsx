import {
  Fragment,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import {
  Button,
  Collapsible,
  CollapsibleContent,
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
  Input,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
  toast,
} from "@ora/ui";
import {
  IconArchive,
  IconChevronDown,
  IconChevronRight,
  IconFolder,
  IconFolderOpen,
  IconGitBranch,
  IconLayoutSidebarLeftCollapse,
  IconMessageCircle,
  IconMessageCirclePlus,
  IconPencil,
  IconPlus,
  IconRoute,
  IconSearch,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import type { Session, Task } from "@ora/contracts";
import type { CurrentUser } from "../../lib/types";
import { UserProfile } from "../sidebar/user-profile";
import { localizeContractError } from "../../i18n/contract-error";
import { useProjects } from "../../state/hooks/use-projects";
import { useTasks } from "../../state/hooks/use-tasks";
import { useSessions } from "../../state/hooks/use-sessions";
import { useRestoreWorkspaceSelection } from "../../state/hooks/use-restore-workspace-selection";
import {
  useRenameWorkflowRun,
  useWorkflowRunsByProject,
} from "../../state/hooks/use-workflow-runs";
import {
  useUpdateProject,
  useUpdateTask,
} from "../../state/hooks/use-workspace-mutations";
import { useStoreWithEqualityFn } from "zustand/traditional";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import {
  draftPlacements,
  draftPlacementsEqual,
  draftSidebarTitle,
  type SessionDraft,
  useDraftSessionsStore,
} from "../../state/stores/draft-sessions-store";
import {
  selectBoundDraftSession,
  startSessionDraft,
  resolveNewChatScope,
} from "../../state/session-drafts";
import { OraMark } from "../../components/ora-mark";
import { DragRegion } from "../../components/drag-region";
import type { GraphWorkflowRunStatus } from "@ora/workflow-runtime";
import { SidebarCreateMenu } from "./sidebar-create-menu";
import { DraftSessionTreeRow } from "./draft-session-tree-row";
import { SessionTreeRow } from "./session-tree-row";
import { useInlineTreeRename } from "./use-inline-tree-rename";

const EMPTY_TASKS: Task[] = [];
const EMPTY_SESSIONS: Session[] = [];
type DraftSearchEntry = Pick<
  SessionDraft,
  "id" | "projectId" | "taskId" | "text"
>;
const EMPTY_DRAFT_SEARCH_ENTRIES: DraftSearchEntry[] = [];

/** Compares only draft fields that can change sidebar search results. */
function draftSearchEntriesEqual(
  left: DraftSearchEntry[],
  right: DraftSearchEntry[],
): boolean {
  if (left.length !== right.length) return false;
  return left.every((entry, index) => {
    const candidate = right[index]!;
    return (
      entry.id === candidate.id &&
      entry.projectId === candidate.projectId &&
      entry.taskId === candidate.taskId &&
      entry.text === candidate.text
    );
  });
}

interface WorkspaceSidebarProps {
  user: CurrentUser;
  onSignOut: () => void;
}

/** Renders projects, direct-chat/worktree tasks, and agent sessions as a dense three-level navigation tree. */
export function WorkspaceSidebar({ user, onSignOut }: WorkspaceSidebarProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const initializedTreeExpansion = useRef(false);

  const projectsQuery = useProjects();
  const tasksQuery = useTasks();
  const sessionsQuery = useSessions();
  // Stabilise the array references so useMemo dependencies don't change every render.
  const projects = useMemo(
    () => projectsQuery.data ?? [],
    [projectsQuery.data],
  );
  const tasks = useMemo(() => tasksQuery.data ?? [], [tasksQuery.data]);
  const sessions = useMemo(
    () => sessionsQuery.data ?? [],
    [sessionsQuery.data],
  );
  // Placement only — text changes must not rebuild the tree.
  // Zustand 5 dropped equality as the hook's 2nd arg; React 19 also requires
  // getSnapshot to return a cached reference when the logical selection is equal.
  const placements = useStoreWithEqualityFn(
    useDraftSessionsStore,
    (s) => draftPlacements(s.drafts),
    draftPlacementsEqual,
  );
  const persistedSessionIds = useMemo(
    () => new Set(sessions.map((session) => session.id)),
    [sessions],
  );
  const visiblePlacements = useMemo(
    () =>
      placements.filter(
        (draft) =>
          draft.pendingSessionId === null ||
          !persistedSessionIds.has(draft.pendingSessionId),
      ),
    [placements, persistedSessionIds],
  );

  useLayoutEffect(() => {
    const draftStore = useDraftSessionsStore.getState();
    const selectedDraftId =
      useWorkspaceSelectionStore.getState().selection.draftId;
    const selectedDraft = draftStore.drafts.find(
      (draft) => draft.id === selectedDraftId,
    );
    // Move selection first so removing a newly persisted draft can never leave
    // draftId pointing at a row that no longer exists.
    if (
      selectedDraft?.pendingSessionId !== null &&
      selectedDraft?.pendingSessionId !== undefined &&
      persistedSessionIds.has(selectedDraft.pendingSessionId)
    ) {
      selectBoundDraftSession({
        projectId: selectedDraft.projectId,
        taskId: selectedDraft.taskId,
        pendingSessionId: selectedDraft.pendingSessionId,
      });
    }
    draftStore.removeCommitted(persistedSessionIds);
  }, [persistedSessionIds]);
  const loading =
    projectsQuery.isPending || tasksQuery.isPending || sessionsQuery.isPending;
  const error = projectsQuery.error ?? tasksQuery.error ?? sessionsQuery.error;

  useRestoreWorkspaceSelection({
    projects,
    tasks,
    sessions,
    // Restore must wait not only while the tree queries are pending but also
    // while any has errored: a settled-but-failed query yields an empty list,
    // which would make the resolver discard a valid candidate as a miss and
    // overwrite the disk selection. Keep the loading indicator on isPending.
    treePending: loading || error !== null,
  });

  const tasksByProjectId = useMemo(() => {
    const grouped = new Map<string, Task[]>();
    for (const task of tasks) {
      if (task.type === "workflow") continue;
      const list = grouped.get(task.projectId);
      if (list) list.push(task);
      else grouped.set(task.projectId, [task]);
    }
    return grouped;
  }, [tasks]);

  const sessionsByTaskId = useMemo(() => {
    const grouped = new Map<string, Session[]>();
    for (const session of sessions) {
      const list = grouped.get(session.taskId);
      if (list) list.push(session);
      else grouped.set(session.taskId, [session]);
    }
    return grouped;
  }, [sessions]);

  const { directDraftsByProjectId, worktreeDraftsByTaskId } = useMemo(() => {
    const direct = new Map<string, typeof visiblePlacements>();
    const worktree = new Map<string, typeof visiblePlacements>();
    for (const draft of visiblePlacements) {
      const grouped =
        draft.taskId === null
          ? { map: direct, key: draft.projectId }
          : { map: worktree, key: draft.taskId };
      const list = grouped.map.get(grouped.key);
      if (list) list.push(draft);
      else grouped.map.set(grouped.key, [draft]);
    }
    return {
      directDraftsByProjectId: direct,
      worktreeDraftsByTaskId: worktree,
    };
  }, [visiblePlacements]);

  const selection = useWorkspaceSelectionStore((s) => s.selection);
  const createFocus = useWorkspaceSelectionStore((s) => s.createFocus);
  const setCreateFocus = useWorkspaceSelectionStore((s) => s.setCreateFocus);
  const selectTask = useWorkspaceSelectionStore((s) => s.selectTask);
  const selectWorkflowRun = useWorkspaceSelectionStore(
    (s) => s.selectWorkflowRun,
  );

  const expandedProjects = useUiStore((s) => s.expandedProjects);
  const expandedTasks = useUiStore((s) => s.expandedTasks);
  const toggleProjectExpand = useUiStore((s) => s.toggleProjectExpand);
  const toggleTaskExpand = useUiStore((s) => s.toggleTaskExpand);
  const setSidebarCollapsed = useUiStore((s) => s.setSidebarCollapsed);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const setDialog = useUiStore((s) => s.setDialog);
  const setDeleteTarget = useUiStore((s) => s.setDeleteTarget);
  const updateProject = useUpdateProject();
  const updateTask = useUpdateTask();

  // Subscribe to structured search data only while filtering. Equality ignores
  // attachment and timestamp churn that cannot change a title match.
  const searchableDrafts = useStoreWithEqualityFn(
    useDraftSessionsStore,
    (s) =>
      needle.length === 0
        ? EMPTY_DRAFT_SEARCH_ENTRIES
        : s.drafts.map(({ id, projectId, taskId, text }) => ({
            id,
            projectId,
            taskId,
            text,
          })),
    draftSearchEntriesEqual,
  );

  const newSessionLabel = t("sidebar.newSession");
  const visibleProjects = useMemo(() => {
    return projects.filter((project) => {
      if (!needle) return true;
      if (project.name.toLowerCase().includes(needle)) return true;
      if (
        searchableDrafts.some(
          (draft) =>
            draft.projectId === project.id &&
            draftSidebarTitle(draft.text, newSessionLabel)
              .toLowerCase()
              .includes(needle),
        )
      ) {
        return true;
      }
      const projectTasks = tasksByProjectId.get(project.id) ?? EMPTY_TASKS;
      return projectTasks.some((task) => {
        if (task.title.toLowerCase().includes(needle)) return true;
        if (
          searchableDrafts.some(
            (draft) =>
              draft.taskId === task.id &&
              draftSidebarTitle(draft.text, newSessionLabel)
                .toLowerCase()
                .includes(needle),
          )
        ) {
          return true;
        }
        const taskSessions = sessionsByTaskId.get(task.id) ?? EMPTY_SESSIONS;
        return taskSessions.some((session) =>
          session.title?.toLowerCase().includes(needle),
        );
      });
    });
  }, [
    needle,
    newSessionLabel,
    projects,
    searchableDrafts,
    sessionsByTaskId,
    tasksByProjectId,
  ]);

  // Expand the initial workspace tree once while preserving later manual collapse choices.
  useEffect(() => {
    if (loading || initializedTreeExpansion.current) return;
    initializedTreeExpansion.current = true;
    useUiStore.setState((state) => ({
      expandedProjects: new Set([
        ...state.expandedProjects,
        ...projects.map((project) => project.id),
      ]),
      expandedTasks: new Set([
        ...state.expandedTasks,
        ...tasks.map((task) => task.id),
      ]),
    }));
  }, [loading, projects, tasks]);

  const openProject = (projectId: string) => {
    // Remember create target without selecting — composer stays on the current chat.
    setCreateFocus({ projectId, taskId: null });
    toggleProjectExpand(projectId);
  };

  /** Same as projects: row click only toggles; New chat follows createFocus. */
  const openTask = (taskId: string, projectId: string) => {
    setCreateFocus({ projectId, taskId });
    toggleTaskExpand(taskId);
  };

  /** Starts a blank chat under create focus, selection, or the first project. */
  const startNewChat = useCallback(() => {
    const scope = resolveNewChatScope(
      useWorkspaceSelectionStore.getState().createFocus,
      useWorkspaceSelectionStore.getState().selection,
      projects[0]?.id ?? null,
      { projects, tasks },
    );
    if (scope === null) {
      setDialog({ kind: "project" });
      return;
    }
    startSessionDraft(scope);
  }, [projects, setDialog, tasks]);

  // Match desktop IDE conventions while preventing the browser's new-window shortcut.
  useEffect(() => {
    const handleNewChatShortcut = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "n") {
        event.preventDefault();
        startNewChat();
      }
    };
    window.addEventListener("keydown", handleNewChatShortcut);
    return () => window.removeEventListener("keydown", handleNewChatShortcut);
  }, [startNewChat]);

  return (
    <>
      {/* Width is owned by the enclosing ResizablePanel, so the aside just fills it. */}
      <aside className="flex size-full min-w-0 flex-col bg-sidebar text-sidebar-foreground">
        <header className="flex h-14 items-center gap-2 px-3">
          <DragRegion>
            <OraMark size="default" />
            <span className="text-[15px] font-semibold tracking-[-0.01em]">
              Ora
            </span>
          </DragRegion>
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => setSidebarCollapsed(true)}
                  aria-label={t("sidebar.collapse")}
                />
              }
            >
              <IconLayoutSidebarLeftCollapse />
            </TooltipTrigger>
            <TooltipContent>{t("sidebar.collapse")}</TooltipContent>
          </Tooltip>
        </header>

        <div className="flex flex-col gap-1 px-2 pb-3">
          <Button
            type="button"
            variant="ghost"
            className="h-9 w-full justify-start gap-2 px-2 text-[13px] font-medium"
            onClick={startNewChat}
          >
            <IconMessageCirclePlus className="size-4 text-muted-foreground" />
            {t("chat.new")}
          </Button>
          <div className="relative min-w-0">
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

        <nav
          className="min-h-0 flex-1 overflow-y-auto px-2 pb-3"
          aria-label={t("sidebar.navigation")}
        >
          <div className="flex h-8 items-center pl-2 pr-1 text-xs font-medium text-muted-foreground">
            <span>{t("sidebar.projects")}</span>
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="ml-auto"
                    onClick={() => setDialog({ kind: "project" })}
                    aria-label={t("sidebar.newProject")}
                  />
                }
              >
                <IconPlus />
              </TooltipTrigger>
              <TooltipContent>{t("sidebar.newProject")}</TooltipContent>
            </Tooltip>
          </div>
          {loading && (
            <p className="px-2 py-6 text-center text-[13px] text-muted-foreground">
              {t("sidebar.loading")}
            </p>
          )}
          {!loading && visibleProjects.length === 0 && (
            <p className="px-2 py-6 text-center text-[13px] text-muted-foreground">
              {t("sidebar.empty")}
            </p>
          )}
          {visibleProjects.map((project) => {
            const projectTasks =
              tasksByProjectId.get(project.id) ?? EMPTY_TASKS;
            const projectSessionIds = projectTasks.flatMap((task) =>
              (sessionsByTaskId.get(task.id) ?? EMPTY_SESSIONS).map(
                (session) => session.id,
              ),
            );
            const projectOpen =
              expandedProjects.has(project.id) || Boolean(needle);
            return (
              <div key={project.id}>
                <TreeRow
                  depth={0}
                  active={
                    selection.projectId === project.id &&
                    selection.taskId === null &&
                    selection.sessionId === null &&
                    selection.draftId === null &&
                    selection.workflowRunId === null
                  }
                  createFocused={
                    createFocus !== null &&
                    createFocus.projectId === project.id &&
                    createFocus.taskId === null &&
                    !(
                      selection.projectId === createFocus.projectId &&
                      selection.taskId === createFocus.taskId
                    )
                  }
                  icon={
                    projectOpen ? (
                      <IconFolderOpen className="size-[18px] text-muted-foreground" />
                    ) : (
                      <IconFolder className="size-[18px] text-muted-foreground" />
                    )
                  }
                  label={project.name}
                  expanded={projectOpen}
                  onClick={() => openProject(project.id)}
                  action={
                    <SidebarCreateMenu
                      projectId={project.id}
                      onNewTask={(projectId) => {
                        startSessionDraft({ projectId, taskId: null });
                      }}
                    />
                  }
                  onRename={(name) =>
                    updateProject.mutateAsync({ project, name })
                  }
                  commands={[
                    {
                      label: t("common.delete"),
                      icon: <IconTrash />,
                      variant: "destructive",
                      onSelect: () =>
                        setDeleteTarget({
                          kind: "project",
                          id: project.id,
                          name: project.name,
                          sessionIds: projectSessionIds,
                        }),
                    },
                  ]}
                />
                <TreeBranch expanded={projectOpen}>
                  <ProjectWorkflowRunRows
                    projectId={project.id}
                    activeRunId={
                      selection.projectId === project.id
                        ? selection.workflowRunId
                        : null
                    }
                    onSelectRun={(runId) =>
                      selectWorkflowRun(runId, project.id)
                    }
                    onDeleteRun={(run) =>
                      setDeleteTarget({
                        kind: "workflowRun",
                        id: run.id,
                        name: run.name,
                        projectId: project.id,
                      })
                    }
                  />
                  {(directDraftsByProjectId.get(project.id) ?? []).map(
                    (draft) => (
                      <DraftSessionTreeRow
                        key={draft.id}
                        draftId={draft.id}
                        depth={1}
                      />
                    ),
                  )}
                  {projectTasks.map((task) => {
                    const taskSessions =
                      sessionsByTaskId.get(task.id) ?? EMPTY_SESSIONS;
                    const taskOpen =
                      expandedTasks.has(task.id) || Boolean(needle);
                    if (task.workspaceMode === "project_root") {
                      const directSession = taskSessions[0];
                      if (directSession) {
                        return (
                          <SessionTreeRow
                            key={task.id}
                            sessionId={directSession.id}
                            taskId={task.id}
                            projectId={project.id}
                            depth={1}
                            title={
                              directSession.title ?? t("sidebar.newSession")
                            }
                            deleteAs="task"
                            workspaceMode={task.workspaceMode}
                          />
                        );
                      }
                      return (
                        <TreeRow
                          key={task.id}
                          depth={1}
                          active={selection.taskId === task.id}
                          icon={
                            <IconMessageCircle
                              className="size-4 text-muted-foreground"
                              aria-label={t("sidebar.directChatTask")}
                            />
                          }
                          label={task.title}
                          onClick={() => selectTask(task.id, task.projectId)}
                          onRename={(name) =>
                            updateTask.mutateAsync({
                              task,
                              title: name,
                            })
                          }
                          commands={[
                            {
                              label: t("common.delete"),
                              icon: <IconTrash />,
                              variant: "destructive",
                              onSelect: () =>
                                setDeleteTarget({
                                  kind: "task",
                                  id: task.id,
                                  name: task.title,
                                  workspaceMode: task.workspaceMode,
                                  sessionIds: [],
                                }),
                            },
                          ]}
                        />
                      );
                    }
                    return (
                      <div key={task.id}>
                        <TreeRow
                          depth={1}
                          active={
                            selection.taskId === task.id &&
                            selection.sessionId === null &&
                            selection.draftId === null
                          }
                          createFocused={
                            createFocus?.taskId === task.id &&
                            !(
                              selection.projectId === createFocus.projectId &&
                              selection.taskId === createFocus.taskId
                            )
                          }
                          icon={
                            <IconGitBranch
                              className="size-4 text-muted-foreground"
                              aria-label={t("sidebar.worktreeTask")}
                            />
                          }
                          label={task.title}
                          expanded={taskOpen}
                          onClick={() => openTask(task.id, task.projectId)}
                          action={
                            <NewSessionButton
                              onClick={() =>
                                startSessionDraft({
                                  projectId: task.projectId,
                                  taskId: task.id,
                                })
                              }
                            />
                          }
                          onRename={(name) =>
                            updateTask.mutateAsync({
                              task,
                              title: name,
                            })
                          }
                          commands={[
                            {
                              label: t("common.delete"),
                              icon: <IconTrash />,
                              variant: "destructive",
                              onSelect: () =>
                                setDeleteTarget({
                                  kind: "task",
                                  id: task.id,
                                  name: task.title,
                                  workspaceMode: task.workspaceMode,
                                  sessionIds: taskSessions.map(
                                    (session) => session.id,
                                  ),
                                }),
                            },
                          ]}
                        />
                        <TreeBranch expanded={taskOpen}>
                          {(worktreeDraftsByTaskId.get(task.id) ?? []).map(
                            (draft) => (
                              <DraftSessionTreeRow
                                key={draft.id}
                                draftId={draft.id}
                                depth={2}
                              />
                            ),
                          )}
                          {taskSessions.map((session) => (
                            <SessionTreeRow
                              key={session.id}
                              sessionId={session.id}
                              taskId={task.id}
                              projectId={project.id}
                              depth={2}
                              title={session.title ?? t("sidebar.newSession")}
                              deleteAs="session"
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

        {error && (
          <p
            data-selectable
            className="border-t border-destructive/20 bg-destructive/10 px-3 py-2 text-xs text-destructive"
          >
            {localizeContractError(error, t)}
          </p>
        )}
        <div className="p-2">
          <UserProfile
            user={user}
            onOpenSettings={() => setSettingsOpen(true)}
            onSignOut={onSignOut}
          />
        </div>
      </aside>
    </>
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
function TreeBranch({
  expanded,
  children,
}: {
  expanded: boolean;
  children: React.ReactNode;
}) {
  return (
    <Collapsible open={expanded}>
      <CollapsibleContent className="h-(--collapsible-panel-height) overflow-hidden transition-[height,opacity] duration-200 ease-out data-ending-style:h-0 data-ending-style:opacity-0 data-starting-style:h-0 data-starting-style:opacity-0">
        {children}
      </CollapsibleContent>
    </Collapsible>
  );
}

interface TreeRowCommand {
  label: string;
  icon: React.ReactNode;
  variant?: "destructive";
  /** Renders a divider above this item, matching session-row delete placement. */
  separatorBefore?: boolean;
  onSelect: () => void;
}

interface TreeRowProps {
  depth: 0 | 1 | 2;
  active: boolean;
  /**
   * Soft highlight for create-focus rows that are not the live selection — shows
   * where New chat will land without implying the composer already switched.
   */
  createFocused?: boolean;
  icon: React.ReactNode;
  label: string;
  meta?: string;
  expanded?: boolean;
  onClick: () => void;
  /** Hover-only plus (create under a project or a new session under a worktree). */
  action?: React.ReactNode;
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
function TreeRow({
  depth,
  active,
  createFocused = false,
  icon,
  label,
  meta,
  expanded,
  onClick,
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

  // Conditional class names are composed with `cn` instead of a nested ternary
  // so each branch reads as a standalone predicate, matching the rest of the UI.
  const rowTone = cn(
    "group/tree flex h-9 items-center rounded-md transition-colors",
    active && "bg-sidebar-accent text-sidebar-accent-foreground",
    !active &&
      createFocused &&
      "bg-sidebar-accent/45 text-sidebar-foreground ring-1 ring-inset ring-sidebar-accent/60",
    !active && !createFocused && "hover:bg-sidebar-accent/70",
  );

  return (
    <div
      className={rowTone}
      data-create-focus={createFocused && !active ? "true" : undefined}
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

/**
 * Hover plus on a worktree: mint/focus a draft under that branch without
 * toggling the row's expand state.
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

/** Per-project run list so each row can call useGraphWorkflowRuns without hook-in-loop. */
function ProjectWorkflowRunRows({
  projectId,
  activeRunId,
  onSelectRun,
  onDeleteRun,
}: {
  projectId: string;
  activeRunId: string | null;
  onSelectRun: (runId: string) => void;
  onDeleteRun: (run: { id: string; name: string }) => void;
}) {
  const { t } = useTranslation();
  const runsQuery = useWorkflowRunsByProject(projectId);
  const renameWorkflowRun = useRenameWorkflowRun();
  const runs = runsQuery.data ?? [];
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
            depth={1}
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
}
