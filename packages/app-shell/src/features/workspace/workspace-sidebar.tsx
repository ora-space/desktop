import {
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
  Input,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@ora/ui";
import {
  IconLayoutSidebarLeftCollapse,
  IconMessageCirclePlus,
  IconPlus,
  IconSearch,
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
import { useStoreWithEqualityFn } from "zustand/traditional";
import { usePersistHydrated } from "../../state/hooks/use-persist-hydrated";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import {
  draftPlacements,
  draftPlacementsEqual,
  draftSidebarTitle,
  type DraftPlacement,
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
import { useStableGroupBy } from "../../lib/use-stable-group-by";
import { ProjectTreeNode } from "./workspace-project-tree-node";

const EMPTY_TASKS: Task[] = [];
const EMPTY_SESSIONS: Session[] = [];
const EMPTY_DRAFTS: DraftPlacement[] = [];
type DraftSearchEntry = Pick<
  SessionDraft,
  "id" | "projectId" | "taskId" | "text"
>;
const EMPTY_DRAFT_SEARCH_ENTRIES: DraftSearchEntry[] = [];

function projectIdOfTask(task: Task): string {
  return task.projectId;
}
function taskIdOfSession(session: Session): string {
  return session.taskId;
}
function projectIdOfDraft(draft: DraftPlacement): string {
  return draft.projectId;
}
function taskIdOfDraft(draft: DraftPlacement): string {
  return draft.taskId!;
}

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
  const uiHydrated = usePersistHydrated(useUiStore.persist);

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
  // Bootstrap and restore both need a successful tree. `!isPending` alone would
  // let a failed/empty fetch miss-clear `pendingRestore` and persist that wipe
  // so the next launch has nothing to restore (flaky "sometimes works").
  const treeReady =
    projectsQuery.isSuccess && tasksQuery.isSuccess && sessionsQuery.isSuccess;
  const error = projectsQuery.error ?? tasksQuery.error ?? sessionsQuery.error;

  const expandTreeKey = useMemo(
    () =>
      JSON.stringify({
        p: projects.map((project) => project.id),
        t: tasks.map((task) => task.id),
      }),
    [projects, tasks],
  );

  useRestoreWorkspaceSelection({
    projects,
    tasks,
    sessions,
    treePending: !treeReady,
  });

  const nonWorkflowTasks = useMemo(
    () => tasks.filter((task) => task.type !== "workflow"),
    [tasks],
  );
  const tasksByProjectId = useStableGroupBy(nonWorkflowTasks, projectIdOfTask);
  const sessionsByTaskId = useStableGroupBy(sessions, taskIdOfSession);

  const directDraftSource = useMemo(
    () => visiblePlacements.filter((draft) => draft.taskId === null),
    [visiblePlacements],
  );
  const worktreeDraftSource = useMemo(
    () => visiblePlacements.filter((draft) => draft.taskId !== null),
    [visiblePlacements],
  );
  const directDraftsByProjectId = useStableGroupBy(
    directDraftSource,
    projectIdOfDraft,
  );
  const worktreeDraftsByTaskId = useStableGroupBy(
    worktreeDraftSource,
    taskIdOfDraft,
  );

  const setSidebarCollapsed = useUiStore((s) => s.setSidebarCollapsed);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const setDialog = useUiStore((s) => s.setDialog);

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

  // First install only: open the whole tree once. After that, trust localStorage
  // (and wait for hydrate so a sync expand-all cannot wipe a restored collapse).
  // Prune whenever the live tree changes so deleted ids do not linger on disk.
  useEffect(() => {
    if (!treeReady || !uiHydrated) return;
    const projectIds = projects.map((project) => project.id);
    const taskIds = tasks.map((task) => task.id);
    const ui = useUiStore.getState();
    if (!ui.treeExpansionBootstrapped) {
      if (initializedTreeExpansion.current) return;
      initializedTreeExpansion.current = true;
      ui.bootstrapTreeExpansion(projectIds, taskIds);
      return;
    }
    ui.pruneTreeExpansion(projectIds, taskIds);
    // `expandTreeKey` gates re-runs so query refetches with the same ids do not
    // rebuild expand sets or churn subscribers.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- ids captured in expandTreeKey
  }, [treeReady, uiHydrated, expandTreeKey]);

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
          {visibleProjects.map((project) => (
            <ProjectTreeNode
              key={project.id}
              project={project}
              tasks={tasksByProjectId.get(project.id) ?? EMPTY_TASKS}
              sessionsByTaskId={sessionsByTaskId}
              directDrafts={
                directDraftsByProjectId.get(project.id) ?? EMPTY_DRAFTS
              }
              worktreeDraftsByTaskId={worktreeDraftsByTaskId}
              forceExpanded={Boolean(needle)}
            />
          ))}
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
