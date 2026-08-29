import type * as acp from "@agentclientprotocol/sdk";
import { IconPlayerSkipForward } from "@tabler/icons-react";
import { useEffect, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import {
  Button,
  Spinner,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@ora/ui";
import type { GraphWorkflowNodeStatus } from "@ora/workflow-runtime";
import { useChatStore } from "../../chat-store-context";
import { useContractsClient } from "../../contracts-client-context";
import { useCompleteWorkflowNode } from "../../state/hooks/use-workflow-runs";
import { useAgents } from "../../state/hooks/use-agents";
import { ChatView } from "../chat/chat-view";
import { expandPromptRoleTokens } from "../chat/expand-prompt-role-tokens";

const RUNNING_SESSION_RELOAD_DELAY_MS = 250;
const NODE_SESSION_CONVERSATION_NAVIGATION = {
  placement: "container",
  minAnchors: 2,
} as const;

interface RunNodeSessionChatProps {
  /** The real Ora session bound to this node. */
  sessionId: string;
  status: GraphWorkflowNodeStatus;
  /** Interactive nodes need these identifiers to expose their explicit completion action. */
  interaction?: { runId: string; nodeId: string };
  /** Actions placed at the right edge of the transcript's trailing slot. */
  sessionActions?: ReactNode;
  /** Reports a confirmed interactive completion so the workspace can follow the next node. */
  onNodeCompleted?: (nodeId: string) => void;
}

/**
 * Renders every workflow-node session through the ordinary chat store and `ChatView`.
 *
 * Ora supplies the first prompt for both node kinds. Interactive nodes additionally expose the
 * ordinary composer and completion control; non-interactive nodes remain transcript-only.
 */
export function RunNodeSessionChat({
  sessionId,
  status,
  interaction,
  sessionActions,
  onNodeCompleted,
}: RunNodeSessionChatProps) {
  const { t } = useTranslation();
  const chatStore = useChatStore();
  const client = useContractsClient();
  const agentsQuery = useAgents();
  // Fetches one role's persona content for prompt expansion; a miss is ordinary.
  const resolveAgentContent = async (agentId: string) => {
    try {
      return (await client.agent.get({ agentId })).agent;
    } catch {
      return undefined;
    }
  };
  const conversation = useStore(
    chatStore,
    (state) => state.conversations[sessionId],
  );
  const complete = useCompleteWorkflowNode();
  const interactive = interaction !== undefined;

  useEffect(() => {
    const existing = chatStore.getState().conversations[sessionId];
    // Workflow attachment can seed an empty, "loaded" conversation before its automatic prompt
    // is published. A missing or incomplete conversation loads immediately.
    if (existing === undefined || (!existing.isLoading && !existing.isLoaded)) {
      void chatStore
        .getState()
        .loadSession(sessionId)
        .catch(() => {});
      return;
    }
    if (
      status !== "running" ||
      existing.isLoading ||
      existing.isResponding ||
      existing.turns.length > 0
    ) {
      return;
    }
    // The backend binds the Session before preparing its automatic prompt so run cancellation can
    // always find it. If the first replay wins that race, retry only while the node is still
    // running and empty; the first recorded prompt or terminal status ends the loop.
    const retry = window.setTimeout(() => {
      void chatStore
        .getState()
        .loadSession(sessionId)
        .catch(() => {});
    }, RUNNING_SESSION_RELOAD_DELAY_MS);
    return () => window.clearTimeout(retry);
  }, [
    chatStore,
    conversation?.isLoaded,
    conversation?.isLoading,
    conversation?.isResponding,
    conversation?.turns.length,
    sessionId,
    status,
  ]);

  const turns = conversation?.turns ?? [];
  const isResponding = conversation?.isResponding ?? false;
  // The session id is persisted before the workflow-owned first prompt is recorded. Empty replay
  // attempts during that gap must remain one stable loading state; otherwise each retry briefly
  // exposes ChatView's empty-history state and makes the node session appear to flash.
  const isWaitingForFirstTurn =
    status === "running" &&
    turns.length === 0 &&
    (conversation?.error ?? null) === null;
  const lastTurn = turns.at(-1);
  const isStreaming = isResponding && (lastTurn?.items.length ?? 0) > 0;
  // Loading a running node follows the workflow-owned first turn. Keep the short
  // load-complete/status-refresh gap disabled through `!isResponding` too; stopping the active
  // turn is session-scoped below and therefore works for both automatic and human prompts.
  const firstTurnInProgress =
    status === "running" &&
    ((conversation?.isLoading ?? true) || !isResponding);
  const active =
    interactive && (status === "awaiting_input" || status === "running");
  const composerDisabled = !active || firstTurnInProgress;
  const completeEnabled =
    interactive &&
    status === "awaiting_input" &&
    !isResponding &&
    !complete.isPending;

  const handleSend = async (text: string, images?: acp.ImageContent[]) => {
    // The transcript keeps the `@role` chip while the agent reads the role's
    // title, description, and persona content from `agentText`. The user's own
    // message stays first so the recorded prompt retains the tokens and history
    // re-renders the chips after a restart.
    const roleExpansion = await expandPromptRoleTokens(
      text,
      agentsQuery.data ?? [],
      resolveAgentContent,
    );
    const agentText =
      roleExpansion === null ? undefined : `${text}\n\n${roleExpansion}`;
    try {
      await chatStore.getState().sendMessage({
        oraSessionId: sessionId,
        text,
        images,
        ...(agentText === undefined ? {} : { agentText }),
      });
    } catch {
      // The send path already recovers the draft; a rejected promise is surfaced
      // by the chat store, so an unhandled rejection must not escape here.
    }
  };
  const handleStop = () => {
    void client.session.cancelPrompt({ sessionId }).catch(() => {});
  };

  return (
    <div className="flex h-full min-h-0 flex-1 overflow-hidden rounded-xl border border-border/70 bg-background">
      <ChatView
        turns={turns}
        modelChanges={conversation?.modelChanges}
        userName={t("account.unknownIdentity")}
        isResponding={isResponding}
        isStreaming={isStreaming}
        isLoading={
          conversation === undefined ||
          conversation.isLoading ||
          isWaitingForFirstTurn
        }
        error={conversation?.error ?? null}
        pendingPermissions={conversation?.pendingPermissions ?? []}
        roles={agentsQuery.data ?? []}
        availableCommands={conversation?.availableCommands ?? []}
        conversationNavigation={NODE_SESSION_CONVERSATION_NAVIGATION}
        disabled={composerDisabled}
        modelSelectorDisabled
        modelSelectorSessionId={sessionId}
        composerVisible={interactive}
        composerActions={
          active || sessionActions != null ? (
            <>
              {active && interaction !== undefined && (
                <Tooltip disabled={completeEnabled || complete.isPending}>
                  <TooltipTrigger render={<span />}>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      className="size-9 rounded-full border border-border/80 bg-background shadow-sm hover:border-primary/30 hover:bg-primary/5"
                      data-testid="complete-current-node"
                      aria-label={t("workflowRun.completeNode.action")}
                      title={t("workflowRun.completeNode.action")}
                      aria-busy={complete.isPending}
                      disabled={!completeEnabled}
                      onClick={() =>
                        complete.mutate(interaction, {
                          onSuccess: () =>
                            onNodeCompleted?.(interaction.nodeId),
                        })
                      }
                    >
                      {complete.isPending ? (
                        <Spinner className="size-4" aria-hidden="true" />
                      ) : (
                        <IconPlayerSkipForward className="size-4" />
                      )}
                    </Button>
                  </TooltipTrigger>
                  {!completeEnabled && (
                    <TooltipContent sideOffset={8}>
                      {t("workflowRun.completeNode.disabledHint")}
                    </TooltipContent>
                  )}
                </Tooltip>
              )}
              {sessionActions}
            </>
          ) : undefined
        }
        onSend={handleSend}
        onStop={handleStop}
        onRespondToPermission={(permissionRequestId, optionId) => {
          void chatStore
            .getState()
            .respondToPermission(sessionId, permissionRequestId, optionId)
            .catch(() => undefined);
        }}
      />
    </div>
  );
}
