import type * as acp from "@agentclientprotocol/sdk";
import { useEffect, useRef, useState } from "react";
import { Button } from "@ora/ui";
import type { AttachSessionResponse, Session, Task } from "@ora/contracts";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import {
  IconBrandGit,
  IconChartBar,
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
import { useSkills } from "../../state/hooks/use-skills";
import {
  useWarmSession,
  warmTargetKey,
} from "../../state/hooks/use-warm-session";
import { queryKeys } from "../../state/hooks/query-keys";
import { useContractsClient } from "../../contracts-client-context";
import { useUiStore } from "../../state/stores/ui-store";
import { useTargetAgentCli } from "../../state/hooks/use-target-agent-cli";
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import {
  buildWorkflowReminder,
  getRun,
  kickNode,
  useWorkflowStore,
  type WorkflowNodeId,
} from "../../state/stores/workflow-store";
import { conversationKeyFor } from "../../state/stores/conversation-key";
import { useComposerPluginSelectionStore } from "../../state/stores/composer-plugin-selection-store";
import { useChatStore } from "../../chat-store-context";
import { DragRegion } from "../../components/drag-region";
import { WindowControls } from "../../components/window-controls";
import { ChatView } from "../chat/chat-view";
import { ComposerContextBar } from "../chat/composer-context-bar";
import { SessionHistoryBanner } from "../chat/session-history-banner";
import { WorkflowStepper } from "../workflow/workflow-stepper";
import { useWorkflowDetection } from "../workflow/use-workflow-detection";
import type { ChatTurn } from "@ora/chat";
import { LocationActionsButton } from "./location-actions-button";
import { WorkflowRunWorkspace } from "../workflow-run/workflow-run-workspace";
import { directChatTitle } from "./workspace-view-utils";
import {
  WorkspaceReviewLayout,
  type WorkspaceReviewContext,
} from "./workspace-review-layout";
import { useTaskDiffLiveSync } from "../../state/hooks/use-task-diff-live-sync";

interface WorkspaceViewProps {
  userName: string;
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

/** Reduces a thrown value to the text shown against a turn that never reached the agent. */
function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Names the chat surface a selection is looking at.
 *
 * Neither half is enough alone: `conversationKeyFor` collapses every project-root
 * chat onto one key, and the warm target does not change when a send adopts its
 * session. Together they move on exactly the two transitions that must retire a
 * pending send — navigating elsewhere, and its own conversation taking over.
 */
function chatSurfaceKeyFor(selection: {
  projectId: string | null;
  taskId: string | null;
  sessionId: string | null;
}): string {
  return `${conversationKeyFor(selection)}|${warmTargetKey(selection) ?? ""}`;
}

/** A sent message shown before its surface has a session to key it under. */
interface PendingSend {
  /** The chat surface it was typed on, so it is retired when that surface is not on screen. */
  surfaceKey: string;
  turn: ChatTurn;
}

/**
 * Builds the turn that stands in for a message sent before this surface holds a
 * warm session id.
 *
 * Nothing can be keyed in the chat store under a session that does not exist
 * yet, so this turn is held by the view instead and is what puts the thread
 * layout, the message and the thinking indicator on screen while the handshake
 * finishes. It mirrors what `sendMessage` materializes so the handover is
 * invisible.
 */
function draftTurn(text: string, images: acp.ImageContent[]): ChatTurn {
  const createdAt = Date.now();
  return {
    id: crypto.randomUUID(),
    userMessage: {
      kind: "message",
      id: crypto.randomUUID(),
      role: "user",
      content: text.trim(),
      ...(images.length === 0
        ? {}
        : {
            structuredContent: images.map((image) => ({
              type: "image" as const,
              ...image,
            })),
          }),
      createdAt,
    },
    items: [],
    status: "streaming",
    stopReason: null,
    error: null,
    createdAt,
  };
}

/** Shows useful project/task context until a session is selected, then opens its agent chat. */
export function WorkspaceView({ userName }: WorkspaceViewProps) {
  const { t } = useTranslation();

  const { data: projects = [] } = useProjects();
  const { data: tasks = [] } = useTasks();
  const sessionsQuery = useSessions();
  const skillsQuery = useSkills();
  const sessions = sessionsQuery.data ?? [];
  const selection = useWorkspaceSelectionStore((s) => s.selection);
  const sidebarCollapsed = useUiStore((s) => s.sidebarCollapsed);
  const setSidebarCollapsed = useUiStore((s) => s.setSidebarCollapsed);
  // Resolved the same way the picker shows it, so the session warmed here is
  // the one the composer and model picker are actually pointing at — a stale
  // read would warm a different agent than what is on screen.
  const targetAgentCli = useTargetAgentCli(selection);
  const openDashboardPanel = useUiStore((s) => s.openDashboardPanel);

  const chatStore = useChatStore();
  useTaskDiffLiveSync(chatStore, sessions);
  const client = useContractsClient();
  const queryClient = useQueryClient();
  // Opens the provider session for this surface before anything is sent, so the
  // model picker has real options and the send path skips the agent handshake.
  const { sessionId: warmSessionId, ensureSessionId } = useWarmSession(
    selection,
    targetAgentCli,
  );

  // A message sent before the handshake lands has no session to be keyed under,
  // so the view carries its turn until one exists. Holding it here is what lets
  // the composer slide down and the message appear on the send itself rather
  // than a round trip later.
  const [pendingSend, setPendingSend] = useState<PendingSend | null>(null);
  // Bumped to abandon a send still waiting on its handshake, which only Stop
  // does — leaving the surface is handled by the key the turn carries. The
  // waiting `dispatchSend` compares the token it captured and gives up.
  const pendingSendToken = useRef(0);

  const project = projects.find((item) => item.id === selection.projectId);
  const task = tasks.find((item) => item.id === selection.taskId);
  const session = sessions.find((item) => item.id === selection.sessionId);
  // Until the first message binds this surface to a persisted session, its
  // conversation lives under the warm one — the same id the composer and the
  // model picker act on. Resolving it the same way here is what lets anything
  // reported before that first send reach the screen.
  const conversationSessionId = selection.sessionId ?? warmSessionId;
  const reviewContext: WorkspaceReviewContext =
    task !== undefined && project !== undefined
      ? { kind: "task", taskId: task.id, projectId: project.id }
      : project !== undefined
        ? { kind: "project", projectId: project.id }
        : { kind: "none" };
  const conversation = useStore(chatStore, (state) =>
    conversationSessionId === null
      ? undefined
      : state.conversations[conversationSessionId],
  );

  // Workflow state is isolated per session (per task before the session exists).
  const workflowKey = conversationKeyFor(selection);
  // Deriving what to show rather than clearing on a transition is what makes the
  // handover atomic: the render that adopts the session both retires this turn
  // and shows the real one, with no frame carrying neither or both.
  const pendingTurn =
    pendingSend?.surfaceKey === chatSurfaceKeyFor(selection)
      ? pendingSend.turn
      : null;
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
    session,
    sessionsQuery,
  ]);

  /**
   * Sends into the selected session, or into the warm session this surface
   * already holds, persisting it against its Task on the way.
   *
   * The warm session's id is final from the moment the chat surface opens, so
   * the optimistic turn is materialized under that id directly and nothing has
   * to be re-keyed afterwards. `displayText` is what the transcript shows while
   * the agent receives `agentText` (used to hide a workflow reminder); images
   * remain structured prompt blocks.
   */
  const dispatchSend = async (
    displayText: string,
    agentText: string | undefined,
    images: acp.ImageContent[] = [],
  ) => {
    const currentKey = conversationKeyFor(
      useWorkspaceSelectionStore.getState().selection,
    );
    if (session) {
      // A move the picker recorded is paid for here rather than when it was
      // chosen: rebinding tears the current agent's connection down, which at
      // click time could have been mid-reply. Running it inside `prepare` means
      // a CLI that refuses the move fails the send it was part of, leaving the
      // message and the pending pick intact to retry.
      // Only the binding decides whether a recorded pick is a move: a record
      // naming the CLI this session already runs on is one the user withdrew by
      // arriving back where they started, and committing it would be refused.
      const recorded = usePendingAgentStore.getState().switches[session.id];
      const pendingSwitch =
        recorded === session.agentCli ? undefined : recorded;
      const prepare =
        pendingSwitch === undefined
          ? undefined
          : async () => {
              const response = await client.session.switchAgent({
                sessionId: session.id,
                agentCli: pendingSwitch,
              });
              usePendingAgentStore.getState().clearPendingSwitch(session.id);
              // The claim consumed the warm entry, so this surface must warm a
              // fresh one rather than keep an id the backend no longer knows.
              queryClient.removeQueries({
                queryKey: queryKeys.warmSession(
                  { type: "task", taskId: session.taskId },
                  pendingSwitch,
                ),
              });
              queryClient.setQueryData<Session[]>(
                queryKeys.sessions,
                (current) => upsertById(current, response.session),
              );
              // Recorded against the session being moved, not the warm one, so
              // the transcript is marked where the move actually takes effect.
              chatStore
                .getState()
                .adoptSwitchedAgent(session.id, response.configOptions);
              return { availableCommands: response.availableCommands };
            };
      try {
        await chatStore.getState().sendMessage({
          oraSessionId: session.id,
          text: displayText,
          agentText,
          images,
          prepare,
        });
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

    // Show the turn before waiting on anything. The handshake can still be open
    // here — the composer never blocks on it — and this is what makes the send
    // land on screen immediately instead of after the round trip.
    const token = (pendingSendToken.current += 1);
    const surfaceKey = chatSurfaceKeyFor(selection);
    setPendingSend({ surfaceKey, turn: draftTurn(displayText, images) });
    let warmed: string | null;
    try {
      warmed = await ensureSessionId();
    } catch (error) {
      // The message never reached an agent, so it stays on screen carrying the
      // failure rather than disappearing with the composer's optimistic clear.
      const message = errorMessage(error);
      if (token !== pendingSendToken.current) return;
      setPendingSend((current) =>
        current === null
          ? null
          : {
              ...current,
              turn: { ...current.turn, status: "failed", error: message },
            },
      );
      throw error;
    }
    // Stopped, or the user moved on while the handshake ran. The surface check is
    // not just cosmetic: proceeding would select the session below and drag the
    // user back to a chat they had left.
    if (token !== pendingSendToken.current) return;
    if (
      chatSurfaceKeyFor(useWorkspaceSelectionStore.getState().selection) !==
      surfaceKey
    ) {
      return;
    }
    if (warmed === null) return;
    // Rebound as a const so the narrowing survives into `prepare` below.
    const sessionId = warmed;

    const projectId = project.id;
    let taskId = task?.id ?? null;
    // The workflow run and the composer's applied plugins both follow the
    // conversation onto its session id, which is now known, so this happens once
    // instead of tracking a moving key.
    useWorkflowStore.getState().rekey(currentKey, sessionId);
    useComposerPluginSelectionStore.getState().rekey(currentKey, sessionId);
    // Point the workspace at this session before anything is awaited, so the
    // optimistic turn is on screen while its task and record are still forming.
    const selectionStore = useWorkspaceSelectionStore.getState();
    if (taskId === null) {
      selectionStore.selectSessionBeforeTask(sessionId, projectId);
    } else {
      selectionStore.selectSession(sessionId, taskId, projectId);
    }
    // `sendMessage` materializes its own turn before its first `await`, so the
    // pending one is released below in the same synchronous stretch. React
    // commits both together: no duplicated message, and no frame back at the
    // landing layout between the two.
    const sent = chatStore.getState().sendMessage({
      oraSessionId: sessionId,
      text: displayText,
      agentText,
      images,
      prepare: async () => {
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
          // Record the owning task straight away. If the attach below fails,
          // the task is retained and the next send reuses it.
          useWorkspaceSelectionStore
            .getState()
            .selectSession(sessionId, taskId, projectId);
        }

        const attachedTaskId = taskId;
        let response: AttachSessionResponse;
        try {
          response = await client.session.attach({
            sessionId,
            taskId: attachedTaskId,
          });
        } finally {
          // The attach attempt consumes the warm entry whether it succeeds or
          // fails, so this surface must warm a fresh one next time rather than
          // keep retrying with an id the backend no longer recognizes.
          queryClient.removeQueries({
            queryKey: queryKeys.warmSession(
              { type: "task", taskId: attachedTaskId },
              targetAgentCli,
            ),
          });
          queryClient.removeQueries({
            queryKey: queryKeys.warmSession(
              { type: "projectRoot", projectId },
              targetAgentCli,
            ),
          });
        }
        queryClient.setQueryData<Session[]>(queryKeys.sessions, (current) =>
          upsertById(current, response.session),
        );
        useUiStore.getState().expandProject(projectId);
        useUiStore.getState().expandTask(attachedTaskId);
        return { availableCommands: response.availableCommands };
      },
    });
    setPendingSend(null);
    try {
      await sent;
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
  const sendOrStartSession = async (
    text: string,
    images: acp.ImageContent[] = [],
  ) => {
    const key = conversationKeyFor(
      useWorkspaceSelectionStore.getState().selection,
    );
    const nodeId = kickNode(getRun(useWorkflowStore.getState(), key));
    let agentText: string | undefined;
    if (nodeId !== null) {
      useWorkflowStore.getState().launchNode(key, nodeId);
      agentText = `${buildWorkflowReminder(nodeId, skillsDir)}\n\n${text}`;
    }
    await dispatchSend(text, agentText, images);
  };

  // Clicking the highlighted stepper node sends its OpenSpec command now, so the
  // agent starts that stage. The transcript shows a short action label while the
  // agent receives the full reminder; the node flips to running.
  const launchWorkflowNode = (id: WorkflowNodeId) => {
    const key = conversationKeyFor(
      useWorkspaceSelectionStore.getState().selection,
    );
    useWorkflowStore.getState().launchNode(key, id);
    const displayText = t("workflow.startNode", {
      node: t(`workflow.node.${id}`),
    });
    void dispatchSend(displayText, buildWorkflowReminder(id, skillsDir)).catch(
      () => undefined,
    );
  };

  // Graph workflow runs own a dedicated workspace branch (D2), not the chat layout.
  if (selection.workflowRunId !== null) {
    return <WorkflowRunWorkspace runId={selection.workflowRunId} />;
  }

  // Anything short of a persisted selected session is a new or optimistic chat.
  const chatIsOpen =
    session === undefined || (task !== undefined && project !== undefined);

  if (chatIsOpen) {
    const canChat = session
      ? session.status === "running" || conversation?.isLoaded === true
      : project !== undefined;
    // A failed background session-create settles onto the draft conversation, so
    // the conversation error already covers the start-up failure path. A pending
    // send has no conversation to settle onto and carries its own.
    const chatError = conversation?.error ?? pendingTurn?.error ?? null;
    // The pending turn is appended rather than pushed into the store, since the
    // session it would be keyed under does not exist while it is on screen. It is
    // what flips `ChatView` out of the landing layout on the send itself.
    const turns =
      pendingTurn === null
        ? (conversation?.turns ?? [])
        : [...(conversation?.turns ?? []), pendingTurn];
    // A pending send is waiting on its handshake, which is exactly the state the
    // thinking indicator describes.
    const isResponding =
      (conversation?.isResponding ?? false) ||
      pendingTurn?.status === "streaming";
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
                  {session.title ?? t("sidebar.newSession")}
                </p>
                {project && task && (
                  <p className="truncate text-[11px] text-muted-foreground">
                    {project.name} / {task.title}
                  </p>
                )}
              </div>
            )}
          </DragRegion>
          <LocationActionsButton
            taskId={task?.id}
            projectPath={project?.rootPath}
          />
          <Button
            variant="ghost"
            size="icon"
            onClick={() => openDashboardPanel("trace")}
            aria-label={t("dashboard.open")}
            title={t("dashboard.open")}
          >
            <IconLayoutDashboard />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            onClick={() => openDashboardPanel("compare")}
            aria-label={t("dashboard.openCompare")}
            title={t("dashboard.openCompare")}
          >
            <IconChartBar />
          </Button>
          <WindowControls />
        </div>
        <SessionHistoryBanner
          session={session}
          notices={conversation?.historyNotices ?? []}
        />
        <WorkspaceReviewLayout context={reviewContext}>
          <ChatView
            taskId={task?.id}
            turns={turns}
            modelChanges={conversation?.modelChanges}
            userName={userName}
            isResponding={isResponding}
            isStreaming={isStreaming}
            isLoading={isLoadingHistory}
            error={chatError}
            pendingPermissions={conversation?.pendingPermissions ?? []}
            skills={skillsQuery.data ?? []}
            availableCommands={conversation?.availableCommands ?? []}
            disabled={!canChat}
            disabledHint={canChat ? undefined : t("chat.pickProject")}
            // A persisted or optimistic session already fixes its project and
            // execution context, so the pickers only belong to a blank composer.
            contextBar={
              selection.sessionId === null && pendingTurn === null ? (
                <ComposerContextBar />
              ) : undefined
            }
            workflowBar={
              <WorkflowStepper
                onLaunch={launchWorkflowNode}
                disabled={!canChat}
              />
            }
            // Failures land in chatError; the rejection itself is expected.
            onSend={(text, images) =>
              void sendOrStartSession(text, images).catch(() => undefined)
            }
            onEmptySubmit={
              quickLaunchNodeId === null
                ? undefined
                : () => launchWorkflowNode(quickLaunchNodeId)
            }
            // A pending send has no session to stop — abandoning it before its
            // handshake resolves is what stops it.
            // Otherwise the selected id, not session.id: during the optimistic
            // startup the real session does not exist yet but the draft key is
            // already live.
            onStop={() => {
              if (pendingTurn !== null) {
                pendingSendToken.current += 1;
                setPendingSend(null);
                return;
              }
              chatStore.getState().stopGeneration(selection.sessionId ?? "");
            }}
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
        </WorkspaceReviewLayout>
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
      <WorkspaceReviewLayout context={reviewContext}>
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
      </WorkspaceReviewLayout>
    </main>
  );
}
