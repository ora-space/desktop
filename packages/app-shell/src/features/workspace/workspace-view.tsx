import type * as acp from "@agentclientprotocol/sdk";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@ora/ui";
import type { AttachSessionResponse, Session } from "@ora/contracts";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import {
  IconBrandGit,
  IconFolder,
  IconGitBranch,
  IconLayoutSidebarLeftExpand,
  IconPlayerPlay,
} from "@tabler/icons-react";
import { useQueryClient } from "@tanstack/react-query";
import { useProjects } from "../../state/hooks/use-projects";
import { useTasks } from "../../state/hooks/use-tasks";
import { useSessions } from "../../state/hooks/use-sessions";
import { useSkills } from "../../state/hooks/use-skills";
import { useAgents } from "../../state/hooks/use-agents";
import { useWorkspaces } from "../../state/hooks/use-workspaces";
import { useWorkspaceCwd } from "../../state/hooks/use-workspace-cwd";
import {
  useWarmSession,
  warmTargetKey,
} from "../../state/hooks/use-warm-session";
import { queryKeys } from "../../state/hooks/query-keys";
import { useContractsClient } from "../../contracts-client-context";
import { useUiStore } from "../../state/stores/ui-store";
import { useTargetAgentCli } from "../../state/hooks/use-target-agent-cli";
import { useTargetAgentReadiness } from "../../state/hooks/use-target-agent-readiness";
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { conversationKeyFor } from "../../state/stores/conversation-key";
import { useComposerPluginSelectionStore } from "../../state/stores/composer-plugin-selection-store";
import { useComposerInputStore } from "../../state/stores/composer-input-store";
import { useDraftSessionsStore } from "../../state/stores/draft-sessions-store";
import {
  recoverFailedDraftSend,
  DraftSendAbandonedError,
  noteComposerSendAdoptedSession,
  reparkDraftComposerContent,
} from "../../state/session-drafts";
import { useChatStore } from "../../chat-store-context";
import { DragRegion } from "../../components/drag-region";
import { WindowControls } from "../../components/window-controls";
import { ChatView } from "../chat/chat-view";
import { expandPromptRoleTokens } from "../chat/expand-prompt-role-tokens";
import { ComposerContextBar } from "../chat/composer-context-bar";
import { SessionAgentBanner } from "../chat/session-agent-banner";
import { SessionHistoryBanner } from "../chat/session-history-banner";
import type { ChatTurn } from "@ora/chat";
import { LocationActionsButton } from "./location-actions-button";
import { SurfaceLauncher } from "../surface/surface-launcher";
import { WorkflowRunWorkspace } from "../workflow-run/workflow-run-workspace";
import { WorkflowEditor } from "../workflow-editor/workflow-editor";
import {
  WorkspaceReviewLayout,
  type WorkspaceReviewContext,
} from "./workspace-review-layout";
import { useWorkspaceDiffLiveSync } from "../../state/hooks/use-workspace-diff-live-sync";

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

/** Reduces a thrown value to the text shown against a turn that never reached the agent. */
function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Names the chat surface a selection is looking at.
 *
 * Neither half is enough alone: `conversationKeyFor` identifies the direct project
 * Workspace while the warm target does not change when a send adopts its
 * session. Together they move on exactly the two transitions that must retire a
 * pending send — navigating elsewhere, and its own conversation taking over.
 */
function chatSurfaceKeyFor(selection: {
  projectId: string | null;
  taskId: string | null;
  sessionId: string | null;
  draftId: string | null;
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
  const { data: workspaces = [] } = useWorkspaces();
  const sessionsQuery = useSessions();
  const skillsQuery = useSkills();
  const agentsQuery = useAgents();
  const sessions = sessionsQuery.data ?? [];
  const selection = useWorkspaceSelectionStore((s) => s.selection);
  const sidebarCollapsed = useUiStore((s) => s.sidebarCollapsed);
  const setSidebarCollapsed = useUiStore((s) => s.setSidebarCollapsed);
  const workflowEditorOpen = useUiStore((s) => s.workflowEditorOpen);
  // Resolved the same way the picker shows it, so the session warmed here is
  // the one the composer and model picker are actually pointing at — a stale
  // read would warm a different agent than what is on screen.
  const targetAgentCli = useTargetAgentCli(selection);
  // Must resolve before the early returns below: hooks cannot sit behind them,
  // and the send gate reads it in the chat branch.
  const targetAgentReadiness = useTargetAgentReadiness(selection);
  const chatStore = useChatStore();
  useWorkspaceDiffLiveSync(chatStore, sessions);
  const client = useContractsClient();
  // Fetches one role's persona content for prompt expansion; a miss is ordinary
  // (the role may have been created before content was required).
  const resolveAgentContent = async (agentId: string) => {
    try {
      return (await client.agent.get({ agentId })).agent;
    } catch {
      return undefined;
    }
  };
  const queryClient = useQueryClient();
  // Opens the provider session for this surface before anything is sent, so the
  // model picker has real options and the send path skips the agent handshake.
  const { sessionId: warmSessionId, ensureSession } = useWarmSession(
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
  const overviewProject = projects.find(
    (item) => item.id === selection.projectId,
  );
  const selectedWorkspace =
    selection.workflowRunId !== null
      ? undefined
      : task !== undefined
        ? workspaces.find((item) => item.id === task.workspaceId)
        : workspaces.find(
            (item) =>
              item.projectId === selection.projectId && item.kind === "main",
          );
  const selectedWorkspaceId =
    selection.workflowRunId !== null
      ? undefined
      : (task?.workspaceId ?? selectedWorkspace?.id);
  const workspaceCwdQuery = useWorkspaceCwd(selectedWorkspaceId);
  const session = sessions.find((item) => item.id === selection.sessionId);
  // Until the first message binds this surface to a persisted session, its
  // conversation lives under the warm one — the same id the composer and the
  // model picker act on. Resolving it the same way here is what lets anything
  // reported before that first send reach the screen.
  const conversationSessionId = selection.sessionId ?? warmSessionId;
  // Memoized: WorkspaceReviewLayout keys per-scope restore off this object's
  // identity, and this component re-renders on every streaming chat update.
  const reviewContext = useMemo<WorkspaceReviewContext>(
    () =>
      task !== undefined && project !== undefined
        ? {
            kind: "task",
            taskId: task.id,
            projectId: project.id,
            workspaceId: task.workspaceId,
          }
        : project !== undefined
          ? {
              kind: "project",
              projectId: project.id,
              workspaceId: selectedWorkspaceId,
            }
          : { kind: "none" },
    [project, task, selectedWorkspaceId],
  );
  const conversation = useStore(chatStore, (state) =>
    conversationSessionId === null
      ? undefined
      : state.conversations[conversationSessionId],
  );

  // Deriving what to show rather than clearing on a transition is what makes the
  // handover atomic: the render that adopts the session both retires this turn
  // and shows the real one, with no frame carrying neither or both.
  const pendingTurn =
    pendingSend?.surfaceKey === chatSurfaceKeyFor(selection)
      ? pendingSend.turn
      : null;

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
    if (targetAgentCli === null) return;
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
        recorded === session.agentRef ? undefined : recorded;
      const prepare =
        pendingSwitch === undefined
          ? undefined
          : async () => {
              const response = await client.session.switchAgent({
                sessionId: session.id,
                agentRef: pendingSwitch,
              });
              usePendingAgentStore.getState().clearPendingSwitch(session.id);
              // The claim consumed the warm entry, so this surface must warm a
              // fresh one rather than keep an id the backend no longer knows.
              queryClient.removeQueries({
                queryKey: queryKeys.warmSession(
                  { type: "workspace", workspaceId: session.workspaceId },
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
            queryKey: queryKeys.workspaceDiffs(session.workspaceId),
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
    let warmed: Awaited<ReturnType<typeof ensureSession>>;
    const draftIdAtSend =
      useWorkspaceSelectionStore.getState().selection.draftId;
    // Hide × for the whole handshake — bind only happens after warm returns.
    if (draftIdAtSend !== null) {
      useDraftSessionsStore.getState().beginSend(draftIdAtSend);
    }
    const endDraftSend = () => {
      if (draftIdAtSend !== null) {
        useDraftSessionsStore.getState().endSend(draftIdAtSend);
      }
    };
    /** Restores a draft-owned payload when Stop or navigation abandons this handshake. */
    const abandonDraftSend = (): void => {
      setPendingSend(null);
      endDraftSend();
      if (draftIdAtSend === null) return;
      reparkDraftComposerContent({
        draftId: draftIdAtSend,
        text: displayText,
        images,
      });
      throw new DraftSendAbandonedError();
    };
    try {
      warmed = await ensureSession();
    } catch (error) {
      // The message never reached an agent, so it stays on screen carrying the
      // failure rather than disappearing with the composer's optimistic clear.
      const message = errorMessage(error);
      if (token !== pendingSendToken.current) {
        // Stop already cleared pendingSend; still drop it if a later path only
        // bumped the token, so returning to this surface cannot resurrect a
        // forever-streaming optimistic turn.
        abandonDraftSend();
        return;
      }
      setPendingSend((current) =>
        current === null
          ? null
          : {
              ...current,
              turn: { ...current.turn, status: "failed", error: message },
            },
      );
      // Composer already cleared locally; re-park so a surface that never left
      // the draft (or the composer's reject handler) can put the text back.
      if (draftIdAtSend !== null) {
        reparkDraftComposerContent({
          draftId: draftIdAtSend,
          text: displayText,
          images,
        });
      }
      endDraftSend();
      throw error;
    }
    // Stopped, or the user moved on while the handshake ran. The surface check is
    // not just cosmetic: proceeding would select the session below and drag the
    // user back to a chat they had left.
    if (token !== pendingSendToken.current) {
      abandonDraftSend();
      return;
    }
    const selectionAfterWarm = useWorkspaceSelectionStore.getState().selection;
    if (
      chatSurfaceKeyFor(selectionAfterWarm) !== surfaceKey ||
      selectionAfterWarm.draftId !== draftIdAtSend
    ) {
      abandonDraftSend();
      return;
    }
    if (warmed === null) {
      setPendingSend(null);
      endDraftSend();
      return;
    }
    // Rebound as a const so the narrowing survives into `prepare` below.
    const sessionId = warmed.sessionId;
    const workspaceId = warmed.workspaceId;
    const draftId = draftIdAtSend;
    const projectId = project.id;
    const taskId = task?.id ?? null;
    // True once attach has written a persisted session. Failures after that
    // belong to the live chat — rolling the draft back would yank the user onto
    // a row removeCommitted may already have deleted.
    let sessionAttached = false;
    try {
      if (draftId !== null) {
        useDraftSessionsStore.getState().bindToSession(draftId, sessionId);
      }
      // Composer hard-fail restore needs the adopted id even after the user
      // navigates away mid-attach; cleared once that catch settles. Keyed by the
      // pre-rekey conversation so project/task landings (no draft) work too.
      noteComposerSendAdoptedSession(currentKey, sessionId);
      // Composer-local stores follow the conversation onto its final session id
      // before the optimistic turn changes surfaces.
      useComposerPluginSelectionStore.getState().rekey(currentKey, sessionId);
      useComposerInputStore.getState().rekey(currentKey, sessionId);
      const selectionStore = useWorkspaceSelectionStore.getState();
      if (taskId === null) {
        selectionStore.selectSessionBeforeTask(sessionId, projectId);
      } else {
        selectionStore.selectSession(sessionId, taskId, projectId);
      }
      // `sendMessage` materializes its own turn before its first `await`, so the
      // pending one is released in the same synchronous stretch without a blank frame.
      const sent = chatStore.getState().sendMessage({
        oraSessionId: sessionId,
        text: displayText,
        agentText,
        images,
        prepare: async () => {
          let response: AttachSessionResponse;
          try {
            response = await client.session.attach({
              sessionId,
              workspaceId,
            });
          } finally {
            // Attach consumes the warm entry on success and failure.
            queryClient.removeQueries({
              queryKey: queryKeys.warmSession(
                { type: "workspace", workspaceId },
                targetAgentCli,
              ),
            });
          }
          // Backend persistence is the recovery boundary: after attach succeeds,
          // rolling back to a client draft could duplicate the real session.
          sessionAttached = true;
          try {
            queryClient.setQueryData<Session[]>(queryKeys.sessions, (current) =>
              upsertById(current, response.session),
            );
          } finally {
            // Even a cache update failure must not leave a muted row pointing at
            // a session that the backend already persisted.
            if (draftId !== null) {
              useDraftSessionsStore.getState().removeCommitted([sessionId]);
            }
          }
          useUiStore.getState().expandProject(projectId);
          if (taskId !== null) {
            useUiStore.getState().expandTask(taskId);
          }
          return { availableCommands: response.availableCommands };
        },
      });
      setPendingSend(null);
      await sent;
    } catch (error) {
      // This catch includes the synchronous bind/rekey/select/send setup. Every
      // pre-attach failure returns ownership to the draft instead of stranding
      // it in sendInFlight.
      if (draftId !== null && !sessionAttached) {
        recoverFailedDraftSend({
          draftId,
          projectId,
          taskId,
          text: displayText,
          images,
          boundSessionId: sessionId,
        });
      }
      throw error;
    } finally {
      endDraftSend();
      await Promise.all([
        sessionsQuery.refetch(),
        queryClient.invalidateQueries({
          queryKey: queryKeys.workspaceDiffs(workspaceId),
        }),
      ]);
    }
  };

  // Composer send. The transcript keeps the `@role` chip while the agent reads
  // the role's title, description, and persona content from `agentText`. The
  // user's own message must stay first in `agentText`: the backend records the
  // prompt as the user turn, so leading with the tokens is what lets history
  // re-render the skill/role chips after a restart.
  const sendOrStartSession = async (
    text: string,
    images: acp.ImageContent[] = [],
  ) => {
    const roleExpansion = await expandPromptRoleTokens(
      text,
      agentsQuery.data ?? [],
      resolveAgentContent,
    );
    const agentText =
      roleExpansion === null ? undefined : `${text}\n\n${roleExpansion}`;
    await dispatchSend(text, agentText, images);
  };

  // Graph workflow definition editor owns the main pane while it is open.
  if (workflowEditorOpen) {
    return <WorkflowEditor />;
  }

  // Graph workflow runs own a dedicated workspace branch (D2), not the chat layout.
  if (selection.workflowRunId !== null) {
    return <WorkflowRunWorkspace runId={selection.workflowRunId} />;
  }

  // Anything short of a persisted selected session is a new or optimistic chat.
  const chatIsOpen = session === undefined || project !== undefined;

  if (chatIsOpen) {
    const canChat =
      targetAgentCli !== null &&
      (session
        ? session.status === "running" || conversation?.isLoaded === true
        : project !== undefined);
    // The send button alone carries this hint: the composer must stay typable
    // so the picker next to it can fix the state, and it only applies where the
    // whole-composer hint does not, so the two bubbles never compete.
    const sendDisabledHint =
      canChat && targetAgentReadiness === "blocked"
        ? t("chat.pickAvailableAgent")
        : undefined;
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
          <LocationActionsButton workspaceId={selectedWorkspaceId} />
          <SurfaceLauncher />
          <WindowControls />
        </div>
        <SessionAgentBanner session={session} />
        <SessionHistoryBanner
          session={session}
          notices={conversation?.historyNotices ?? []}
        />
        <WorkspaceReviewLayout context={reviewContext}>
          <ChatView
            taskId={task?.id}
            projectId={project?.id}
            workspaceId={selectedWorkspaceId}
            turns={turns}
            modelChanges={conversation?.modelChanges}
            userName={userName}
            isResponding={isResponding}
            isStreaming={isStreaming}
            isLoading={isLoadingHistory}
            error={chatError}
            pendingPermissions={conversation?.pendingPermissions ?? []}
            skills={skillsQuery.data ?? []}
            roles={agentsQuery.data ?? []}
            availableCommands={conversation?.availableCommands ?? []}
            disabled={!canChat}
            // An untouched chat cannot send yet, but disabling this picker as
            // part of the composer would make choosing its first agent impossible.
            modelSelectorDisabled={targetAgentCli !== null && !canChat}
            disabledHint={
              canChat
                ? undefined
                : project === undefined
                  ? t("chat.pickProject")
                  : t("chat.pickAgent")
            }
            sendDisabledHint={sendDisabledHint}
            // A persisted or optimistic session already fixes its project and
            // execution context, so the pickers only belong to a blank composer.
            contextBar={
              selection.sessionId === null && pendingTurn === null ? (
                <ComposerContextBar />
              ) : undefined
            }
            // Failures land in chatError; the rejection also lets the composer
            // restore unsent text when the surface never left the draft.
            onSend={(text, images) => sendOrStartSession(text, images)}
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
        <LocationActionsButton workspaceId={selectedWorkspaceId} />
        <SurfaceLauncher />
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
              {task?.title ??
                overviewProject?.name ??
                t("workspace.defaultTitle")}
            </h1>
            <p className="mt-2 max-w-md text-sm leading-6 text-muted-foreground">
              {task
                ? t("workspace.taskHint")
                : overviewProject
                  ? t("workspace.projectHint")
                  : t("workspace.emptyHint")}
            </p>
            {(overviewProject || task) && (
              <div className="mt-6 grid gap-px overflow-hidden rounded-md border border-border bg-border sm:grid-cols-2">
                <div className="bg-background p-4">
                  <div className="flex items-center gap-2 text-xs text-muted-foreground">
                    <IconBrandGit className="size-4" />
                    {t("workspace.repository")}
                  </div>
                  <p className="mt-2 truncate text-sm font-medium">
                    {workspaceCwdQuery.data ?? overviewProject?.name}
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
                            (item) => item.workspaceId === task.workspaceId,
                          ).length,
                        })
                      : t("workspace.worktreeCount", {
                          count: tasks.filter(
                            (item) => item.projectId === overviewProject?.id,
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
