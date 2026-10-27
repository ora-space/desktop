import type * as acp from "@agentclientprotocol/sdk";
import {
  IconAlertTriangle,
  IconCheck,
  IconPlayerSkipForward,
} from "@tabler/icons-react";
import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Badge,
  Button,
  Spinner,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
} from "@ora/ui";
import type {
  GraphWorkflowNodeStatus,
  WorkflowNodeCompletion,
  WorkflowNodeCompletionBlockerKind,
} from "@ora/workflow-runtime";
import { useChatStore } from "../../chat-store-context";
import { useContractsClient } from "../../contracts-client-context";
import { useContractErrorToast } from "../../i18n/use-contract-error-toast";
import { useCompleteWorkflowNode } from "../../state/data/workflow-runs";
import { useAgents } from "../../state/hooks/use-agents";
import { ChatView } from "../chat/chat-view";
import { expandPromptRoleTokens } from "../chat/expand-prompt-role-tokens";
import { isAwaitingConfirmation } from "./run-node-completion";

const RUNNING_SESSION_RELOAD_DELAY_MS = 250;
const NODE_SESSION_CONVERSATION_NAVIGATION = {
  placement: "container",
  minAnchors: 2,
} as const;

/** Conservative preview for node runs that predate `payload.completion`; never ready. */
const UNKNOWN_COMPLETION: WorkflowNodeCompletion = {
  blockers: [{ kind: "turn_unknown", params: {} }],
  warnings: [],
  provisional: false,
  ready: false,
};

/** Everything the step card needs from the owning workflow act. */
interface RunNodeSessionInteraction {
  runId: string;
  nodeId: string;
  /** Persisted completion preview of the parked turn; absent on older node runs. */
  completion?: WorkflowNodeCompletion;
  /** Direct successor titles; empty means this is the last step. */
  nextNodeTitles?: string[];
}

interface RunNodeSessionChatProps {
  /** The real Ora session bound to this node. */
  sessionId: string;
  status: GraphWorkflowNodeStatus;
  /** Interactive nodes need these identifiers to expose their explicit completion action. */
  interaction?: RunNodeSessionInteraction;
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
  const showContractError = useContractErrorToast();
  const interactive = interaction !== undefined;
  /** Force completion stays behind an explicit confirmation because it can advance half a turn. */
  const [forceOpen, setForceOpen] = useState(false);
  const completion = interaction?.completion ?? UNKNOWN_COMPLETION;
  // A node parked before completion snapshots existed has no preview to gate on. It keeps the
  // ordinary completion entry: the backend materializes the snapshot from the session history and
  // applies the same blockers gate, returning `workflow_node_incomplete` when the turn is not
  // finished. Guessing readiness here would either block a valid completion or hide the check.
  const hasCompletionSnapshot = interaction?.completion !== undefined;
  const completionReady = hasCompletionSnapshot && completion.ready === true;
  // A provisional snapshot is the park path's guess before the turn settled. Its blockers are not
  // final: `prepare` re-derives the same turn and repairs the snapshot when it can, so the card must
  // offer the ordinary completion instead of sending the user to a forced one.
  const completionProvisional =
    hasCompletionSnapshot && completion.provisional === true;
  // The workflow's opening turn always parks unconfirmed, so the node waits for the user instead
  // of offering to advance: the agent may have ended that reply with a question. It is not
  // "paused" either — the turn settled normally — so the card keeps the awaiting-input badge and
  // only offers the explicit continue / complete-anyway actions.
  const completionAwaitingConfirmation = isAwaitingConfirmation(
    interaction?.completion,
  );
  /** The card can still offer a normal completion while the snapshot is unknown or provisional. */
  const completionActionable =
    completionReady || completionProvisional || !hasCompletionSnapshot;
  /** Ready, still settling, or waiting for the user's confirmation of the opening turn. */
  const completionAwaiting =
    completionReady || completionProvisional || completionAwaitingConfirmation;

  useEffect(() => {
    const existing = chatStore.getState().conversations[sessionId];
    // Plugin / runtime faults must not retry forever: each load toggles
    // isLoading, remounts the transcript, and jitters Theater. Ordinary chat
    // already stops once `error` is set; keep the same brake here.
    if (existing?.error != null) {
      return;
    }
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
    conversation?.error,
    conversation?.isLoaded,
    conversation?.isLoading,
    conversation?.isResponding,
    conversation?.turns.length,
    sessionId,
    status,
  ]);

  const turns = conversation?.turns ?? [];
  const isResponding = conversation?.isResponding ?? false;
  const loadFailed = (conversation?.error ?? null) !== null;
  // The session id is persisted before the workflow-owned first prompt is recorded. Empty replay
  // attempts during that gap must remain one stable loading state; otherwise each retry briefly
  // exposes ChatView's empty-history state and makes the node session appear to flash.
  const isWaitingForFirstTurn =
    status === "running" && turns.length === 0 && !loadFailed;
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
    !complete.isPending &&
    completionActionable;

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

  /** Sends one human turn with a scoped instruction; the backend adds the step boundary. */
  const handleContinue = async (ready: boolean) => {
    const prompt = ready
      ? t("workflowRun.stepCard.continueReadyPrompt")
      : completionAwaitingConfirmation
        ? t("workflowRun.stepCard.continueUnconfirmedPrompt")
        : t("workflowRun.stepCard.continuePrompt", {
            reason: completionReasonText(completion, t),
          });
    await handleSend(prompt);
  };

  /** Commits completion from the card or the right-bottom action; force must be confirmed. */
  const handleComplete = (force: boolean) => {
    if (interaction === undefined) {
      return;
    }
    complete.mutate(
      {
        runId: interaction.runId,
        nodeId: interaction.nodeId,
        force,
      },
      {
        onSuccess: () => onNodeCompleted?.(interaction.nodeId),
        // A rejected completion is a user-visible outcome, not a refresh: a stale turn or a
        // changed output must say so instead of silently leaving the card unchanged.
        onError: (error) => {
          showContractError(error);
        },
      },
    );
  };

  const stepCard = (() => {
    if (!interactive || status !== "awaiting_input") {
      return undefined;
    }
    const nextNodeTitles = interaction?.nextNodeTitles ?? [];
    const nextNodeLabel = nextNodeTitles.join(
      t("workflowRun.stepCard.nextSeparator"),
    );
    const blockedKindSet = new Set(
      completion.blockers.map((blocker) => blocker.kind),
    );
    const blockedKinds = [...blockedKindSet];
    const warningTexts = [...new Set(completion.warnings)].map((warning) =>
      completionWarningText(warning, t),
    );
    const actionDisabled = isResponding || complete.isPending;
    return (
      <section
        data-testid="node-step-card"
        aria-label={t("workflowRun.stepCard.title")}
        className="rounded-xl border border-border/80 bg-card px-3 py-2.5 shadow-[0_1px_3px_rgba(0,0,0,0.05)]"
      >
        <div className="flex items-center justify-between gap-2">
          <p className="text-[11px] font-medium uppercase tracking-[0.04em] text-muted-foreground">
            {t("workflowRun.stepCard.title")}
          </p>
          <Badge
            variant="outline"
            data-paused={completionAwaiting ? undefined : ""}
            className={cn(
              "text-[10px]",
              completionAwaiting
                ? "border-amber-500/35 bg-amber-500/10 text-amber-900 dark:text-amber-200"
                : "border-border bg-muted/60 text-muted-foreground",
            )}
          >
            {t(
              completionAwaiting
                ? "workflowRun.stepCard.waitingBadge"
                : "workflowRun.stepCard.pausedBadge",
            )}
          </Badge>
        </div>
        {completionReady ? (
          <p className="mt-1.5 text-xs text-muted-foreground">
            {t("workflowRun.stepCard.readyHint")}
          </p>
        ) : completionProvisional ? (
          <p className="mt-1.5 text-xs text-muted-foreground">
            {t("workflowRun.stepCard.provisional")}
          </p>
        ) : completionAwaitingConfirmation ? (
          <>
            <p className="mt-1.5 text-xs font-medium">
              {t("workflowRun.stepCard.unconfirmedTitle")}
            </p>
            <p className="mt-1 text-[11px] text-muted-foreground/80">
              {t("workflowRun.stepCard.unconfirmedHint")}
            </p>
          </>
        ) : hasCompletionSnapshot ? (
          <>
            <p className="mt-1.5 text-xs font-medium">
              {t("workflowRun.stepCard.blockedTitle")}
            </p>
            <ul className="mt-1 space-y-0.5">
              {blockedKinds.map((kind) => (
                <li
                  key={kind}
                  className="flex items-start gap-1.5 text-xs text-muted-foreground"
                >
                  <IconAlertTriangle
                    className="mt-0.5 size-3.5 shrink-0 text-amber-600 dark:text-amber-400"
                    aria-hidden="true"
                  />
                  <span>{t(completionBlockerReasonKey(kind))}</span>
                </li>
              ))}
            </ul>
            <p className="mt-1.5 text-[11px] text-muted-foreground/80">
              {t("workflowRun.stepCard.blockedHint")}
            </p>
          </>
        ) : (
          <>
            <p className="mt-1.5 text-xs font-medium">
              {t("workflowRun.stepCard.legacyTitle")}
            </p>
            <p className="mt-1 text-[11px] text-muted-foreground/80">
              {t("workflowRun.stepCard.legacyHint")}
            </p>
          </>
        )}
        {warningTexts.length > 0 && (
          <ul className="mt-1.5 space-y-0.5">
            {warningTexts.map((warning) => (
              <li
                key={warning}
                className="text-[11px] text-muted-foreground/80"
              >
                {warning}
              </li>
            ))}
          </ul>
        )}
        <div className="mt-2.5 flex flex-wrap items-center gap-2">
          {completionReady ? (
            <>
              <Button
                type="button"
                size="sm"
                data-testid="complete-ready-node"
                disabled={actionDisabled}
                aria-busy={complete.isPending}
                onClick={() => handleComplete(false)}
              >
                {complete.isPending ? (
                  <Spinner className="size-3.5" aria-hidden="true" />
                ) : (
                  <IconCheck className="size-3.5" aria-hidden="true" />
                )}
                {nextNodeTitles.length > 0
                  ? t("workflowRun.stepCard.readyAction", {
                      next: nextNodeLabel,
                    })
                  : t("workflowRun.stepCard.readyLastAction")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                data-testid="continue-chat"
                disabled={actionDisabled}
                onClick={() => void handleContinue(true)}
              >
                {t("workflowRun.stepCard.continueChat")}
              </Button>
            </>
          ) : completionProvisional ? (
            <>
              <Button
                type="button"
                size="sm"
                data-testid="complete-ready-node"
                disabled={actionDisabled}
                aria-busy={complete.isPending}
                onClick={() => handleComplete(false)}
              >
                {complete.isPending ? (
                  <Spinner className="size-3.5" aria-hidden="true" />
                ) : (
                  <IconCheck className="size-3.5" aria-hidden="true" />
                )}
                {nextNodeTitles.length > 0
                  ? t("workflowRun.stepCard.readyAction", {
                      next: nextNodeLabel,
                    })
                  : t("workflowRun.stepCard.readyLastAction")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                data-testid="continue-current-step"
                disabled={actionDisabled}
                onClick={() => void handleContinue(false)}
              >
                {t("workflowRun.stepCard.continueStep")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                data-testid="force-complete-node"
                disabled={actionDisabled}
                onClick={() => setForceOpen(true)}
              >
                {t("workflowRun.stepCard.forceAction")}
              </Button>
            </>
          ) : hasCompletionSnapshot ? (
            <>
              <Button
                type="button"
                size="sm"
                data-testid="continue-current-step"
                disabled={actionDisabled}
                onClick={() => void handleContinue(false)}
              >
                {t("workflowRun.stepCard.continueStep")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                data-testid="force-complete-node"
                disabled={actionDisabled}
                onClick={() => setForceOpen(true)}
              >
                {t("workflowRun.stepCard.forceAction")}
              </Button>
            </>
          ) : (
            <>
              <Button
                type="button"
                size="sm"
                data-testid="complete-legacy-node"
                disabled={actionDisabled}
                aria-busy={complete.isPending}
                onClick={() => handleComplete(false)}
              >
                {complete.isPending ? (
                  <Spinner className="size-3.5" aria-hidden="true" />
                ) : (
                  <IconCheck className="size-3.5" aria-hidden="true" />
                )}
                {t("workflowRun.stepCard.legacyAction")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                data-testid="continue-current-step"
                disabled={actionDisabled}
                onClick={() => void handleContinue(false)}
              >
                {t("workflowRun.stepCard.continueStep")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                data-testid="force-complete-node"
                disabled={actionDisabled}
                onClick={() => setForceOpen(true)}
              >
                {t("workflowRun.stepCard.forceAction")}
              </Button>
            </>
          )}
        </div>
      </section>
    );
  })();

  return (
    <div className="flex h-full min-h-0 flex-1 overflow-hidden rounded-xl border border-border/70 bg-background">
      <ChatView
        sessionId={sessionId}
        turns={turns}
        modelChanges={conversation?.modelChanges}
        userName={t("account.unknownIdentity")}
        isResponding={isResponding}
        isStreaming={isStreaming}
        isLoading={
          !loadFailed &&
          turns.length === 0 &&
          (conversation === undefined ||
            conversation.isLoading ||
            isWaitingForFirstTurn)
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
        trailing={stepCard}
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
                      onClick={() => handleComplete(false)}
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
                      {completionReady
                        ? t("workflowRun.completeNode.disabledHint")
                        : completionAwaitingConfirmation
                          ? t("workflowRun.completeNode.unconfirmedHint")
                          : t("workflowRun.completeNode.blockedHint")}
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
      {interactive && (
        <AlertDialog open={forceOpen} onOpenChange={setForceOpen}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>
                {t("workflowRun.stepCard.forceTitle")}
              </AlertDialogTitle>
              <AlertDialogDescription>
                {completionAwaitingConfirmation
                  ? t("workflowRun.stepCard.forceDescriptionUnconfirmed")
                  : t("workflowRun.stepCard.forceDescription")}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel disabled={complete.isPending}>
                {t("common.cancel")}
              </AlertDialogCancel>
              <AlertDialogAction
                data-testid="confirm-force-complete"
                variant="destructive"
                disabled={complete.isPending}
                onClick={() => {
                  setForceOpen(false);
                  handleComplete(true);
                }}
              >
                {t("workflowRun.stepCard.forceConfirm")}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      )}
    </div>
  );
}

/** Stable translation key for one completion blocker; unknown kinds stay conservatively blocking. */
function completionBlockerReasonKey(
  kind: WorkflowNodeCompletionBlockerKind,
): string {
  return `workflowRun.stepCard.blocker.${kind}`;
}

/** Maps known non-blocking warning codes; unknown backend details stay visible as data. */
function completionWarningText(
  warning: string,
  translate: (key: string) => string,
): string {
  if (warning === "empty_output") {
    return translate("workflowRun.stepCard.warning.empty_output");
  }
  if (warning === "history_unavailable") {
    return translate("workflowRun.stepCard.warning.history_unavailable");
  }
  if (warning === "turn_not_settled") {
    return translate("workflowRun.stepCard.warning.turn_not_settled");
  }
  if (warning.startsWith("history_integrity_gap")) {
    return translate("workflowRun.stepCard.warning.history_integrity_gap");
  }
  return warning;
}

/** Human-readable reason list used inside the scoped continue instruction. */
function completionReasonText(
  completion: WorkflowNodeCompletion,
  translate: (key: string) => string,
): string {
  const kinds = [
    ...new Set(completion.blockers.map((blocker) => blocker.kind)),
  ];
  if (kinds.length === 0) {
    return translate("workflowRun.stepCard.blocker.turn_unknown");
  }
  return kinds
    .map((kind) => translate(completionBlockerReasonKey(kind)))
    .join(" ");
}
