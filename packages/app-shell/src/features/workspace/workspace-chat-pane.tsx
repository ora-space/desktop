import type * as acp from "@agentclientprotocol/sdk";
import type { ChatTurn } from "@ora/chat";
import type {
  Agent,
  Session,
  SessionHistoryNotice,
  Skill,
} from "@ora/contracts";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import { useChatStore } from "../../chat-store-context";
import type { AgentEffectReadiness } from "../../state/hooks/use-agent-effect-readiness";
import type { AgentReadiness } from "../../state/hooks/use-target-agent-readiness";
import { localizeAgentEffectFailureSummary } from "../chat/agent-effect-failure-reason";
import { ChatView } from "../chat/chat-view";
import { ComposerContextBar } from "../chat/composer-context-bar";
import { SessionHistoryBanner } from "../chat/session-history-banner";
import type { SessionSetupPresentation } from "../chat/session-setup";
import {
  SessionUsageIndicator,
  shouldShowSessionUsage,
} from "../chat/session-usage-surface";

const NO_HISTORY_NOTICES: SessionHistoryNotice[] = [];
const NO_SETUPS: SessionSetupPresentation[] = [];

/**
 * Usage arrives on the same conversation object as each streamed token.
 * Reading it here keeps that telemetry off the workspace shell.
 */
export function SessionUsageSlot({ sessionId }: { sessionId: string | null }) {
  const chatStore = useChatStore();
  const usage = useStore(chatStore, (state) =>
    sessionId === null ? undefined : state.conversations[sessionId]?.usage,
  );
  if (usage === undefined || !shouldShowSessionUsage(usage)) return null;
  return <SessionUsageIndicator usage={usage} />;
}

/**
 * History notices change when a replay reports a gap, not when a token arrives.
 * A missing conversation must return one stable empty list: a fresh `[]` on
 * every store update would redraw this banner for each token.
 */
export function SessionHistoryNotices({
  session,
}: {
  session: Session | undefined;
}) {
  const chatStore = useChatStore();
  const sessionId = session?.id ?? null;
  const notices = useStore(chatStore, (state) => {
    if (sessionId === null) return NO_HISTORY_NOTICES;
    return state.conversations[sessionId]?.historyNotices ?? NO_HISTORY_NOTICES;
  });
  return <SessionHistoryBanner session={session} notices={notices} />;
}

interface WorkspaceChatPaneProps {
  sessionId: string | null;
  session: Session | undefined;
  taskId?: string;
  projectId?: string;
  workspaceId?: string;
  userName: string;
  pendingTurn: ChatTurn | null;
  pendingSetup: SessionSetupPresentation | null;
  sessionSetups: SessionSetupPresentation[];
  skills: Skill[];
  roles: Agent[];
  targetAgentCli: string | null;
  projectSelected: boolean;
  /** Resolved folder path when the workspace checkout is gone; null when present or unknown. */
  missingDirectory: string | null;
  agentEffectReadiness: AgentEffectReadiness;
  targetAgentReadiness: AgentReadiness;
  showContextBar: boolean;
  onSend: (text: string, images?: acp.ImageContent[]) => void;
  onStop: () => void;
  onRespondToPermission: (
    permissionRequestId: string,
    optionId: string,
  ) => void;
}

/**
 * The only workspace child that subscribes to the live conversation.
 *
 * Each streamed token replaces that conversation object. Subscribing here means
 * the chat thread reconciles, while the workspace shell and the review panel
 * beside it keep the render they already committed.
 */
export function WorkspaceChatPane({
  sessionId,
  session,
  taskId,
  projectId,
  workspaceId,
  userName,
  pendingTurn,
  pendingSetup,
  sessionSetups,
  skills,
  roles,
  targetAgentCli,
  projectSelected,
  missingDirectory,
  agentEffectReadiness,
  targetAgentReadiness,
  showContextBar,
  onSend,
  onStop,
  onRespondToPermission,
}: WorkspaceChatPaneProps) {
  const { t } = useTranslation();
  const chatStore = useChatStore();
  const conversation = useStore(chatStore, (state) =>
    sessionId === null ? undefined : state.conversations[sessionId],
  );
  const canChat =
    targetAgentCli !== null &&
    (session
      ? session.status === "running" || conversation?.isLoaded === true
      : projectSelected);
  // The send button alone carries this hint: the composer must stay typable
  // so the picker next to it can fix the state, and it only applies where the
  // whole-composer hint does not, so the two bubbles never compete.
  // A missing folder outranks every agent-side reason: none of them can clear while the
  // checkout is gone, and they would point the user at the agent instead of the folder.
  const sendDisabledHint =
    canChat && missingDirectory !== null
      ? t("chat.workspaceDirectoryMissing")
      : canChat && agentEffectReadiness.kind === "failed"
        ? t("chat.agentEffectsFailed", {
            message: localizeAgentEffectFailureSummary(agentEffectReadiness, t),
          })
        : canChat && agentEffectReadiness.kind === "blocked"
          ? t("chat.agentEffectsNotReady")
          : canChat && targetAgentReadiness === "blocked"
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
  // Session setup owns its own status row; once prompting begins, the ordinary
  // response indicator starts from the turn's later response-phase timestamp.
  const isResponding =
    (conversation?.isResponding ?? false) ||
    pendingTurn?.status === "streaming";
  const visibleSessionSetups =
    pendingTurn !== null
      ? pendingSetup === null
        ? NO_SETUPS
        : [pendingSetup]
      : sessionSetups;
  const lastTurn = conversation?.turns.at(-1);
  // Output has begun once the live turn carries any item; until then the turn is
  // still starting up (session creation or the wait for the first token).
  const isStreaming =
    (conversation?.isResponding ?? false) && (lastTurn?.items.length ?? 0) > 0;
  // A selected session always owns a thread, so treat it as loading until its
  // history has landed (or failed). This also covers the render between selecting
  // the session and loadSession flipping isLoading on — without it the composer
  // would bounce back to the landing layout for a frame when switching sessions.
  const isLoadingHistory =
    session !== undefined &&
    conversation?.isLoaded !== true &&
    conversation?.error == null;

  return (
    <ChatView
      sessionId={sessionId ?? undefined}
      taskId={taskId}
      projectId={projectId}
      workspaceId={workspaceId}
      turns={turns}
      modelChanges={conversation?.modelChanges}
      userName={userName}
      isResponding={isResponding}
      sessionSetups={visibleSessionSetups}
      isStreaming={isStreaming}
      isLoading={isLoadingHistory}
      error={chatError}
      pendingPermissions={conversation?.pendingPermissions ?? []}
      skills={skills}
      roles={roles}
      availableCommands={conversation?.availableCommands ?? []}
      disabled={!canChat}
      // An untouched chat cannot send yet, but disabling this picker as
      // part of the composer would make choosing its first agent impossible.
      modelSelectorDisabled={targetAgentCli !== null && !canChat}
      disabledHint={
        canChat
          ? undefined
          : projectSelected
            ? t("chat.pickAgent")
            : t("chat.pickProject")
      }
      sendDisabledHint={sendDisabledHint}
      // A persisted or optimistic session already fixes its project and
      // execution context, so the pickers only belong to a blank composer.
      contextBar={showContextBar ? <ComposerContextBar /> : undefined}
      // Failures land in chatError; the rejection also lets the composer
      // restore unsent text when the surface never left the draft.
      onSend={onSend}
      // A pending send has no session to stop — abandoning it before its
      // handshake resolves is what stops it. The parent owns that token.
      onStop={onStop}
      onRespondToPermission={onRespondToPermission}
    />
  );
}
