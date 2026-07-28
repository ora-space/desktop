import { useEffect } from "react";
import { Button } from "@ora/ui";
import type { Session, Task } from "@ora/contracts";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import {
  IconBrandGit,
  IconFolder,
  IconGitBranch,
  IconLayoutDashboard,
  IconLayoutSidebarLeftExpand,
  IconPlayerPlay,
} from "@tabler/icons-react";
import { useQueryClient } from "@tanstack/react-query";
import { useProjects } from "../../state/hooks/use-projects";
import { useTasks } from "../../state/hooks/use-tasks";
import { useSessions } from "../../state/hooks/use-sessions";
import { queryKeys } from "../../state/hooks/query-keys";
import { useContractsClient } from "../../contracts-client-context";
import { useUiStore } from "../../state/stores/ui-store";
import { useSettingsStore } from "../../state/stores/settings-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import {
  buildWorkflowReminder,
  getRun,
  kickNode,
  useWorkflowStore,
  workflowKeyFor,
  type WorkflowNodeId,
} from "../../state/stores/workflow-store";
import { useChatStore } from "../../chat-store-context";
import { DragRegion } from "../../components/drag-region";
import { WindowControls } from "../../components/window-controls";
import { ChatView } from "../chat/chat-view";
import { ComposerContextBar } from "../chat/composer-context-bar";
import { WorkflowStepper } from "../workflow/workflow-stepper";
import { useWorkflowDetection } from "../workflow/use-workflow-detection";
import type { ChatTurn } from "@ora/chat";
import { LocationActionsButton } from "./location-actions-button";
import { agentCliLabel } from "./agent-cli";
import { TaskChangesLayout } from "../diff/task-changes-layout";
import { useTaskDiffLiveSync } from "../../state/hooks/use-task-diff-live-sync";

interface WorkspaceViewProps {
  userName: string;
}

/** Builds a compact direct-chat title from the first message without splitting Unicode characters. */
export function directChatTitle(text: string): string {
  const normalized = text.trim().replace(/\s+/gu, " ");
  return Array.from(normalized).slice(0, 10).join("");
}

/** Inserts a freshly-created entity into query data before the invalidation refetch completes. */
function upsertById<T extends { id: string }>(
  current: T[] | undefined,
  entity: T,
): T[] {
  return [...(current ?? []).filter((item) => item.id !== entity.id), entity];
}

/** Stable empty-turns reference so the workflow detection effect does not re-run each render. */
const EMPTY_TURNS: ChatTurn[] = [];

/** Shows useful project/task context until a session is selected, then opens its agent chat. */
export function WorkspaceView({ userName }: WorkspaceViewProps) {
  const { t } = useTranslation();

  const { data: projects = [] } = useProjects();
  const { data: tasks = [] } = useTasks();
  const sessionsQuery = useSessions();
  const sessions = sessionsQuery.data ?? [];
  const selection = useWorkspaceSelectionStore((s) => s.selection);
  const sidebarCollapsed = useUiStore((s) => s.sidebarCollapsed);
  const setSidebarCollapsed = useUiStore((s) => s.setSidebarCollapsed);
  const settingsAgentCli = useSettingsStore((s) => s.settings.agentCli);
  const setDashboardOpen = useUiStore((s) => s.setDashboardOpen);

  const chatStore = useChatStore();
  useTaskDiffLiveSync(chatStore, sessions);
  const client = useContractsClient();
  const queryClient = useQueryClient();

  const project = projects.find((item) => item.id === selection.projectId);
  const task = tasks.find((item) => item.id === selection.taskId);
  const session = sessions.find((item) => item.id === selection.sessionId);
  const conversation = useStore(chatStore, (state) =>
    selection.sessionId === null
      ? undefined
      : state.conversations[selection.sessionId],
  );

  // Workflow state is isolated per session (per task before the session exists).
  const workflowKey = workflowKeyFor(selection);
  // Absolute path to the OpenSpec skills, so the agent finds them from its worktree
  // cwd even when `.opencode/skills` lives only at the project root.
  const skillsDir =
    project === undefined
      ? ".opencode/skills"
      : `${project.rootPath.replace(/[\\/]+$/, "")}/.opencode/skills`;
  // The highlighted (blue) stage, if any, so pressing Enter on an empty composer
  // launches it directly.
  const workflowRun = useWorkflowStore((state) => getRun(state, workflowKey));
  const quickLaunchNodeId = kickNode(workflowRun);
  // Best-effort: reflect any OpenSpec status JSON the agent emits into the stepper.
  useWorkflowDetection(workflowKey, conversation?.turns ?? EMPTY_TURNS);

  useEffect(() => {
    if (
      session !== undefined &&
      conversation?.isLoading !== true &&
      conversation?.isLoaded !== true &&
      conversation?.error == null
    ) {
      // A browser refresh replaces the in-memory chat store without stopping the backend-owned
      // process, so a selected session can still be Running while its local history is empty.
      void chatStore
        .getState()
        .loadSession(session.id)
        .then(() => sessionsQuery.refetch())
        .catch(() => undefined);
    }
  }, [
    chatStore,
    conversation?.error,
    conversation?.isLoaded,
    conversation?.isLoading,
    session?.id,
    session?.status,
    sessionsQuery,
  ]);

  /**
   * Sends into the selected session, or lazily creates the selected execution
   * context first. The new-session path is optimistic: the store materializes
   * the user turn up front (so the composer slides into the thread immediately)
   * and creates the agent session in the background, re-pointing selection at
   * the draft and then the real id as each becomes available.
   *
   * Core send: shows `displayText` in the transcript while the agent receives
   * `agentText` (used to hide a workflow reminder). `currentKey` tracks the
   * workflow run as an optimistic session moves from its task key to draft to
   * real id.
   */
  const dispatchSend = async (
    displayText: string,
    agentText: string | undefined,
  ) => {
    let currentKey = workflowKeyFor(
      useWorkspaceSelectionStore.getState().selection,
    );
    if (session) {
      try {
        await chatStore
          .getState()
          .sendMessage({ oraSessionId: session.id, text: displayText, agentText });
      } finally {
        // Connection failures can stop the provider process, so refresh the persisted
        // lifecycle snapshot after every finite prompt without polling idle sessions.
        await Promise.all([
          sessionsQuery.refetch(),
          queryClient.invalidateQueries({
            queryKey: queryKeys.taskDiffs(session.taskId),
          }),
        ]);
      }
      return;
    }
    if (project === undefined) return;

    const projectId = project.id;
    let taskId = task?.id ?? null;
    let draftSessionId: string | null = null;
    try {
      await chatStore.getState().sendMessage({
        text: displayText,
        agentText,
        createSession: async () => {
          if (taskId === null) {
            const response = await client.task.create({
              projectId,
              title: directChatTitle(displayText),
              status: "todo",
              workspaceMode: "project_root",
            });
            const createdTask = response.task;
            taskId = createdTask.id;
            queryClient.setQueryData<Task[]>(queryKeys.tasks, (current) =>
              upsertById(current, createdTask),
            );
            void queryClient.invalidateQueries({ queryKey: queryKeys.tasks });
            useUiStore.getState().expandProject(projectId);

            // Keep the optimistic conversation visible while the slower provider
            // session handshake runs. If that handshake fails, this real task is
            // retained and the next send reuses it.
            if (draftSessionId !== null) {
              useWorkspaceSelectionStore
                .getState()
                .selectSession(draftSessionId, createdTask.id, projectId);
            }
          }

          const response = await client.session.create({
            taskId,
            agentCli: settingsAgentCli,
          });
          queryClient.setQueryData<Session[]>(queryKeys.sessions, (current) =>
            upsertById(current, response.session),
          );
          return response.session.id;
        },
        // Show the optimistic turn under its temporary key right away, and move
        // the workflow run onto that key so it follows the session as it forms.
        onDraft: (draftId) => {
          draftSessionId = draftId;
          useWorkflowStore.getState().rekey(currentKey, draftId);
          currentKey = draftId;
          const selectionStore = useWorkspaceSelectionStore.getState();
          if (taskId === null) {
            selectionStore.selectDraftSession(draftId, projectId);
          } else {
            selectionStore.selectSession(draftId, taskId, projectId);
          }
        },
        // The store has already re-keyed the conversation onto the real id, so
        // selecting it here cannot flash an empty thread.
        onSessionCreated: (realSessionId) => {
          useWorkflowStore.getState().rekey(currentKey, realSessionId);
          currentKey = realSessionId;
          void queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
          useWorkspaceSelectionStore
            .getState()
            .selectSession(realSessionId, taskId!, projectId);
          useUiStore.getState().expandProject(projectId);
          useUiStore.getState().expandTask(taskId!);
        },
      });
    } finally {
      await Promise.all([
        sessionsQuery.refetch(),
        taskId === null
          ? Promise.resolve()
          : queryClient.invalidateQueries({
              queryKey: queryKeys.taskDiffs(taskId),
            }),
      ]);
    }
  };

  // Composer send. In Spec mode, a message typed while a stage is highlighted (none
  // running) launches that stage and rides its reminder; the reminder shows only in
  // `agentText`, never the transcript. Within a running stage nothing is injected.
  const sendOrStartSession = async (text: string) => {
    const key = workflowKeyFor(useWorkspaceSelectionStore.getState().selection);
    const nodeId = kickNode(getRun(useWorkflowStore.getState(), key));
    let agentText: string | undefined;
    if (nodeId !== null) {
      useWorkflowStore.getState().launchNode(key, nodeId);
      agentText = `${buildWorkflowReminder(nodeId, skillsDir)}\n\n${text}`;
    }
    await dispatchSend(text, agentText);
  };

  // Clicking the highlighted stepper node sends its OpenSpec command now, so the
  // agent starts that stage. The transcript shows a short action label while the
  // agent receives the full reminder; the node flips to running.
  const launchWorkflowNode = (id: WorkflowNodeId) => {
    const key = workflowKeyFor(useWorkspaceSelectionStore.getState().selection);
    useWorkflowStore.getState().launchNode(key, id);
    const displayText = t("workflow.startNode", { node: t(`workflow.node.${id}`) });
    void dispatchSend(displayText, buildWorkflowReminder(id, skillsDir)).catch(() => undefined);
  };

  // Anything short of a persisted selected session is a new or optimistic chat.
  const chatIsOpen =
    session === undefined || (task !== undefined && project !== undefined);

  if (chatIsOpen) {
    const canChat = session
      ? session.status === "running" || conversation?.isLoaded === true
      : project !== undefined;
    // A failed background session-create settles onto the draft conversation, so
    // the conversation error already covers the start-up failure path.
    const chatError = conversation?.error ?? null;
    const lastTurn = conversation?.turns.at(-1);
    // Output has begun once the live turn carries any item; until then the turn is
    // still starting up (session creation or the wait for the first token).
    const isStreaming =
      (conversation?.isResponding ?? false) &&
      (lastTurn?.items.length ?? 0) > 0;
    // A selected session always owns a thread, so treat it as loading until its
    // history has landed (or failed). This also covers the render between selecting
    // the session and loadSession flipping isLoading on — without it the composer
    // would bounce back to the landing layout for a frame when switching sessions.
    const isLoadingHistory =
      session !== undefined &&
      conversation?.isLoaded !== true &&
      conversation?.error == null;
    return (
      <main
        id="main-content"
        className="relative flex min-h-0 min-w-0 flex-1 flex-col bg-background"
      >
        <div className="flex h-14 shrink-0 items-center gap-2 px-3 sm:px-4">
          {sidebarCollapsed && (
            <Button
              variant="ghost"
              size="icon"
              onClick={() => setSidebarCollapsed(false)}
              aria-label={t("sidebar.expand")}
            >
              <IconLayoutSidebarLeftExpand />
            </Button>
          )}
          <DragRegion>
            {session && (
              <div className="min-w-0">
                <p className="truncate text-sm font-medium tracking-[-0.01em]">
                  {agentCliLabel(session.agentCli)}
                </p>
                {project && task && (
                  <p className="truncate text-[11px] text-muted-foreground">
                    {project.name} / {task.title}
                  </p>
                )}
              </div>
            )}
          </DragRegion>
          <LocationActionsButton taskId={task?.id} projectPath={project?.rootPath} />
          <Button variant="ghost" size="icon" onClick={() => setDashboardOpen(true)} aria-label={t("dashboard.open")} title={t("dashboard.open")}>
            <IconLayoutDashboard />
          </Button>
          <WindowControls />
        </div>
        <TaskChangesLayout taskId={task?.id}>
          <ChatView
            turns={conversation?.turns ?? []}
            userName={userName}
            isResponding={conversation?.isResponding ?? false}
            isStreaming={isStreaming}
            isLoading={isLoadingHistory}
            error={chatError}
            pendingPermissions={conversation?.pendingPermissions ?? []}
            disabled={!canChat}
            disabledHint={canChat ? undefined : t("chat.pickProject")}
            // A persisted or optimistic session already fixes its project and
            // execution context, so the pickers only belong to a blank composer.
            contextBar={
              selection.sessionId === null ? <ComposerContextBar /> : undefined
            }
            workflowBar={
              <WorkflowStepper onLaunch={launchWorkflowNode} disabled={!canChat} />
            }
            // Failures land in chatError; the rejection itself is expected.
            onSend={(text) =>
              void sendOrStartSession(text).catch(() => undefined)
            }
            onEmptySubmit={
              quickLaunchNodeId === null
                ? undefined
                : () => launchWorkflowNode(quickLaunchNodeId)
            }
            // The selected id, not session.id: during the optimistic startup the
            // real session does not exist yet but the draft key is already live.
            onStop={() =>
              chatStore.getState().stopGeneration(selection.sessionId ?? "")
            }
            onRespondToPermission={(permissionRequestId, optionId) => {
              if (session) {
                void chatStore
                  .getState()
                  .respondToPermission(
                    session.id,
                    permissionRequestId,
                    optionId,
                  )
                  .catch(() => undefined);
              }
            }}
          />
        </TaskChangesLayout>
      </main>
    );
  }

  return (
    <main
      id="main-content"
      className="flex min-h-0 min-w-0 flex-1 flex-col bg-background"
    >
      <header className="flex h-14 items-center border-b border-border px-3">
        {sidebarCollapsed && (
          <Button
            variant="ghost"
            size="icon"
            onClick={() => setSidebarCollapsed(false)}
            aria-label={t("sidebar.expand")}
          >
            <IconLayoutSidebarLeftExpand />
          </Button>
        )}
        <DragRegion>
          <span className="text-[13px] font-medium text-muted-foreground">
            {t("workspace.overview")}
          </span>
        </DragRegion>
        <LocationActionsButton
          taskId={task?.id}
          projectPath={project?.rootPath}
        />
        <WindowControls />
      </header>
      <TaskChangesLayout taskId={task?.id}>
        <div className="flex min-h-0 flex-1 items-center justify-center p-6">
          <section className="w-full max-w-xl">
            <div className="mb-6 flex size-11 items-center justify-center rounded-lg border border-border bg-muted">
              {task ? (
                <IconGitBranch className="size-5 text-sky-600" />
              ) : (
                <IconFolder className="size-5 text-amber-600" />
              )}
            </div>
            <h1 className="text-xl font-semibold">
              {task?.title ?? project?.name ?? t("workspace.defaultTitle")}
            </h1>
            <p className="mt-2 max-w-md text-sm leading-6 text-muted-foreground">
              {task
                ? t("workspace.taskHint")
                : project
                  ? t("workspace.projectHint")
                  : t("workspace.emptyHint")}
            </p>
            {(project || task) && (
              <div className="mt-6 grid gap-px overflow-hidden rounded-md border border-border bg-border sm:grid-cols-2">
                <div className="bg-background p-4">
                  <div className="flex items-center gap-2 text-xs text-muted-foreground">
                    <IconBrandGit className="size-4" />
                    {t("workspace.repository")}
                  </div>
                  <p className="mt-2 truncate text-sm font-medium">
                    {project?.rootPath}
                  </p>
                </div>
                <div className="bg-background p-4">
                  <div className="flex items-center gap-2 text-xs text-muted-foreground">
                    <IconPlayerPlay className="size-4" />
                    {t("workspace.agentSessions")}
                  </div>
                  <p className="mt-2 text-sm font-medium">
                    {task
                      ? t("workspace.sessionCount", {
                          count: sessions.filter(
                            (item) => item.taskId === task.id,
                          ).length,
                        })
                      : t("workspace.worktreeCount", {
                          count: tasks.filter(
                            (item) => item.projectId === project?.id,
                          ).length,
                        })}
                  </p>
                </div>
              </div>
            )}
          </section>
        </div>
      </TaskChangesLayout>
    </main>
  );
}
