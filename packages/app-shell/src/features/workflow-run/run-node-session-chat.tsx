import type * as acp from "@agentclientprotocol/sdk";
import { RemoteContractError } from "@ora/contracts";
import {
  IconAlertTriangle,
  IconCheck,
  IconPlayerSkipForward,
} from "@tabler/icons-react";
import { useCallback, useEffect, useState, type ReactNode } from "react";
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
  WorkflowNodeCompletionBlocker,
  WorkflowNodeCompletionBlockerKind,
  WorkflowNodeStopCause,
} from "@ora/workflow-runtime";
import type { WorkflowNodeIncompleteParams } from "@ora/contracts";
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
  needsConfirmation: false,
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
  /**
   * Blockers from the latest rejected completion, tagged with the turn they describe. File checks
   * run only when completion is clicked, so these cannot live in the park snapshot; showing them
   * here reuses the same blocker reasons the card already renders while keeping a ready snapshot.
   * Tagging the turn lets a later poll clear the rejection by derivation instead of an effect.
   */
  const [rejection, setRejection] = useState<{
    identity: string;
    blockers: WorkflowNodeCompletionBlocker[];
  }>({ identity: "", blockers: [] });
  const completion = interaction?.completion ?? UNKNOWN_COMPLETION;
  // A node parked before completion snapshots existed has no preview to gate on. It keeps the
  // ordinary completion entry: the backend materializes the snapshot from the session history and
  // applies the same blockers gate, returning `workflow_node_incomplete` when the turn is not
  // finished. Guessing readiness here would either block a valid completion or hide the check.
  const hasCompletionSnapshot = interaction?.completion !== undefined;
  const completionReady = hasCompletionSnapshot && completion.ready === true;
  // A provisional snapshot is the park path's guess before the turn settled. Its blockers are not
  // final: `prepare` re-derives the same turn and repairs the snapshot when it can, so the card
  // keeps offering completion, but it never enables the right-bottom advancement action.
  const completionProvisional =
    hasCompletionSnapshot && completion.provisional === true;
  // The workflow's opening turn always parks unconfirmed, so the node waits for the user instead
  // of offering to advance: the agent may have ended that reply with a question. The backend
  // classifies this gate; only `confirmInitialTurn` lifts it and `force` cannot stand in for it.
  const completionNeedsConfirmation = isAwaitingConfirmation(
    interaction?.completion,
  );
  // An agent-policy turn whose accepted signal waits for the user's approval, or one that has not
  // signaled yet. The backend materializes both gates into blockers and the card reads those.
  const completionAgentConfirmation =
    interaction?.completion?.requiresAgentConfirmation === true;
  const completionAwaitingAgentSignal =
    interaction?.completion?.awaitsAgentSignal === true;
  // The opening turn's own confirmation, read from the materialized blockers rather than from the
  // single-blocker shortcut: an agent-policy first turn also carries the signal blocker.
  const openingTurnUnconfirmed = completion.blockers.some(
    (blocker) => blocker.kind === "turn_unconfirmed",
  );
  const completionParkKind = hasCompletionSnapshot
    ? completion.parkKind
    : undefined;
  // A user stop stays "awaiting input": the user chose to stop, so it is not a system failure.
  const completionUserStopped =
    hasCompletionSnapshot &&
    !completionProvisional &&
    !completionNeedsConfirmation &&
    !completionAwaitingAgentSignal &&
    completionParkKind === "awaiting_user";
  // Timeout, truncation, refusal, unknown end, session crash, or an output-contract failure.
  const completionAbnormal =
    hasCompletionSnapshot &&
    !completionProvisional &&
    !completionNeedsConfirmation &&
    completionParkKind === "abnormal";
  // Snapshots written before `parkKind` existed keep the existing paused presentation.
  const completionLegacy =
    hasCompletionSnapshot &&
    !completionReady &&
    !completionProvisional &&
    !completionNeedsConfirmation &&
    completionParkKind === undefined;
  /** Badges that mean the node still belongs to the user's participation. */
  const completionAwaitingUser =
    completionReady ||
    completionProvisional ||
    completionNeedsConfirmation ||
    completionAgentConfirmation ||
    completionAwaitingAgentSignal ||
    completionUserStopped;
  /** A ready snapshot, an accepted agent signal, or the legacy snapshot-less entry is actionable. */
  const completionActionable =
    completionReady || completionAgentConfirmation || !hasCompletionSnapshot;

  // A new turn or another node makes the previous rejection stale; keep it while the same turn
  // is polled again so a ready snapshot still explains why the last click was refused.
  const completionTurnIdentity = `${interaction?.nodeId ?? ""}:${completion.turnKey?.userSeq ?? ""}:${completion.turnKey?.endedSeq ?? ""}`;
  const rejectionBlockers =
    rejection.identity === completionTurnIdentity ? rejection.blockers : [];

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
      : completionNeedsConfirmation
        ? t("workflowRun.stepCard.continueUnconfirmedPrompt")
        : completionUserStopped
          ? t("workflowRun.stepCard.continueUserStoppedPrompt")
          : completionAbnormal
            ? t("workflowRun.stepCard.continueAbnormalPrompt", {
                reason: completionParkReasonTexts(completion, t).join(
                  t("workflowRun.stepCard.reasonSeparator"),
                ),
              })
            : t("workflowRun.stepCard.continuePrompt", {
                reason: completionBlockerReasonText(completion, t),
              });
    await handleSend(prompt);
  };

  /**
   * Commits completion from the card or the right-bottom action.
   *
   * Force must always be confirmed by the dialog. `confirmInitialTurn` is the ordinary
   * confirmation of the workflow's opening turn: a normal completion that never records the
   * forced audit and therefore never travels with `force`.
   */
  const handleComplete = (force: boolean, confirmInitialTurn = false) => {
    if (interaction === undefined) {
      return;
    }
    complete.mutate(
      {
        runId: interaction.runId,
        nodeId: interaction.nodeId,
        force,
        ...(confirmInitialTurn ? { confirmInitialTurn: true } : {}),
      },
      {
        onSuccess: () => {
          setRejection({
            identity: completionTurnIdentity,
            blockers: [],
          });
          onNodeCompleted?.(interaction.nodeId);
        },
        // A rejected completion is a user-visible outcome, not a refresh: a stale turn or a
        // changed output must say so instead of silently leaving the card unchanged.
        onError: (error) => {
          if (
            error instanceof RemoteContractError &&
            error.code === "workflow_node_incomplete"
          ) {
            const params = error.payload.params as WorkflowNodeIncompleteParams;
            setRejection({
              identity: completionTurnIdentity,
              blockers: params.blockers.map((blocker) => ({
                kind: blocker.kind,
                params:
                  blocker.params?.path === undefined
                    ? {}
                    : { path: blocker.params.path },
              })),
            });
          }
          showContractError(error);
        },
      },
    );
  };

  /** Confirms the accepted agent signal the user is looking at; stale ids are rejected. */
  const handleCompleteAgent = useCallback(() => {
    if (interaction === undefined) {
      return;
    }
    const signalId = interaction.completion?.agentSignal?.signalId;
    if (signalId === undefined) {
      return;
    }
    // A first-turn signal still needs the opening turn's own explicit confirmation; the backend
    // rejects the signal alone so one click cannot stand in for both gates.
    complete.mutate(
      {
        runId: interaction.runId,
        nodeId: interaction.nodeId,
        force: false,
        ...(openingTurnUnconfirmed ? { confirmInitialTurn: true } : {}),
        confirmAgentCompletion: { signalId },
      },
      {
        onSuccess: () => onNodeCompleted?.(interaction.nodeId),
        onError: (error) => {
          showContractError(error);
        },
      },
    );
  }, [
    interaction,
    complete,
    onNodeCompleted,
    showContractError,
    openingTurnUnconfirmed,
  ]);

  const stepCard = (() => {
    if (!interactive || status !== "awaiting_input") {
      return undefined;
    }
    const nextNodeTitles = interaction?.nextNodeTitles ?? [];
    const nextNodeLabel = nextNodeTitles.join(
      t("workflowRun.stepCard.nextSeparator"),
    );
    const nextActionLabel =
      nextNodeTitles.length > 0
        ? t("workflowRun.stepCard.readyAction", { next: nextNodeLabel })
        : t("workflowRun.stepCard.readyLastAction");
    const confirmActionLabel =
      nextNodeTitles.length > 0
        ? t("workflowRun.stepCard.confirmInitialAction", {
            next: nextNodeLabel,
          })
        : t("workflowRun.stepCard.confirmInitialLastAction");
    const blockedKindSet = new Set(
      completion.blockers.map((blocker) => blocker.kind),
    );
    const blockedKinds = [...blockedKindSet];
    const warningTexts = [...new Set(completion.warnings)].map((warning) =>
      completionWarningText(warning, t),
    );
    /** Stop cause / contract reasons shown by the classified park states. */
    const parkReasonTexts = completionParkReasonTexts(completion, t);
    const actionDisabled = isResponding || complete.isPending;
    // A provisional workflow-origin snapshot still asks for the explicit confirmation; a
    // provisional human turn keeps the ordinary completion it has always offered.
    const provisionalNeedsConfirmation =
      completionProvisional && completion.turnOrigin === "workflow";
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
            data-paused={
              completionAwaitingUser || completionAbnormal ? undefined : ""
            }
            data-abnormal={completionAbnormal ? "" : undefined}
            className={cn(
              "text-[10px]",
              completionAbnormal
                ? "border-destructive/35 bg-destructive/10 text-destructive"
                : completionAwaitingUser
                  ? "border-amber-500/35 bg-amber-500/10 text-amber-900 dark:text-amber-200"
                  : "border-border bg-muted/60 text-muted-foreground",
            )}
          >
            {t(
              completionAbnormal
                ? "workflowRun.stepCard.abnormalBadge"
                : completionAwaitingUser
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
        ) : completionNeedsConfirmation && !completionAgentConfirmation ? (
          <>
            <p className="mt-1.5 text-xs font-medium">
              {t("workflowRun.stepCard.unconfirmedTitle")}
            </p>
            <p className="mt-1 text-xs text-muted-foreground">
              {t("workflowRun.stepCard.unconfirmedReason")}
            </p>
            <p className="mt-1 text-[11px] text-muted-foreground/80">
              {t("workflowRun.stepCard.unconfirmedHint")}
            </p>
          </>
        ) : completionAgentConfirmation ? (
          <>
            <p className="mt-1.5 text-xs font-medium">
              {t("workflowRun.stepCard.agentConfirmationTitle")}
            </p>
            <p className="mt-1 text-xs text-muted-foreground">
              {t("workflowRun.stepCard.agentConfirmationReason")}
            </p>
            <p className="mt-1 text-[11px] text-muted-foreground/80">
              {t("workflowRun.stepCard.agentConfirmationHint")}
            </p>
          </>
        ) : completionAwaitingAgentSignal ? (
          <>
            <p className="mt-1.5 text-xs font-medium">
              {t("workflowRun.stepCard.agentSignalTitle")}
            </p>
            <p className="mt-1 text-xs text-muted-foreground">
              {t("workflowRun.stepCard.agentSignalReason")}
            </p>
            <p className="mt-1 text-[11px] text-muted-foreground/80">
              {t("workflowRun.stepCard.agentSignalHint")}
            </p>
          </>
        ) : completionUserStopped ? (
          <>
            <ul className="mt-1.5 space-y-0.5">
              {(parkReasonTexts.length > 0
                ? parkReasonTexts
                : [t("workflowRun.stepCard.stopCause.user_stopped")]
              ).map((reason) => (
                <li key={reason} className="text-xs font-medium">
                  {reason}
                </li>
              ))}
            </ul>
            <p className="mt-1 text-[11px] text-muted-foreground/80">
              {t("workflowRun.stepCard.userStoppedHint")}
            </p>
          </>
        ) : completionAbnormal ? (
          <>
            <ul className="mt-1.5 space-y-0.5">
              {parkReasonTexts.map((reason) => (
                <li
                  key={reason}
                  className="flex items-start gap-1.5 text-xs text-muted-foreground"
                >
                  <IconAlertTriangle
                    className="mt-0.5 size-3.5 shrink-0 text-destructive"
                    aria-hidden="true"
                  />
                  <span>{reason}</span>
                </li>
              ))}
            </ul>
            <p className="mt-1.5 text-[11px] text-muted-foreground/80">
              {t("workflowRun.stepCard.abnormalHint")}
            </p>
          </>
        ) : completionLegacy ? (
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
        {rejectionBlockers.length > 0 && (
          <div className="mt-1.5">
            <p className="text-xs font-medium">
              {t("workflowRun.stepCard.completionRejectedTitle")}
            </p>
            <ul className="mt-1 space-y-0.5">
              {rejectionBlockers.map((blocker, index) => (
                <li
                  key={`${blocker.kind}:${index}`}
                  className="flex items-start gap-1.5 text-xs text-muted-foreground"
                >
                  <IconAlertTriangle
                    className="mt-0.5 size-3.5 shrink-0 text-amber-600 dark:text-amber-400"
                    aria-hidden="true"
                  />
                  <span>
                    {t(completionBlockerReasonKey(blocker.kind), {
                      path:
                        typeof blocker.params?.path === "string"
                          ? blocker.params.path
                          : "",
                    })}
                  </span>
                </li>
              ))}
            </ul>
          </div>
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
                {nextActionLabel}
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
          ) : completionNeedsConfirmation &&
            !completionAgentConfirmation &&
            !completionAwaitingAgentSignal ? (
            <>
              <Button
                type="button"
                size="sm"
                data-testid="confirm-initial-turn"
                disabled={actionDisabled}
                aria-busy={complete.isPending}
                onClick={() => handleComplete(false, true)}
              >
                {complete.isPending ? (
                  <Spinner className="size-3.5" aria-hidden="true" />
                ) : (
                  <IconCheck className="size-3.5" aria-hidden="true" />
                )}
                {confirmActionLabel}
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
            </>
          ) : completionAgentConfirmation ? (
            <>
              <Button
                type="button"
                size="sm"
                data-testid="confirm-agent-completion"
                disabled={actionDisabled}
                aria-busy={complete.isPending}
                onClick={handleCompleteAgent}
              >
                {complete.isPending ? (
                  <Spinner className="size-3.5" aria-hidden="true" />
                ) : (
                  <IconCheck className="size-3.5" aria-hidden="true" />
                )}
                {t("workflowRun.stepCard.confirmAgentAction")}
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
            </>
          ) : completionAwaitingAgentSignal ? (
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
          ) : completionProvisional ? (
            <>
              {provisionalNeedsConfirmation ? (
                <Button
                  type="button"
                  size="sm"
                  data-testid="confirm-initial-turn"
                  disabled={actionDisabled}
                  aria-busy={complete.isPending}
                  onClick={() => handleComplete(false, true)}
                >
                  {complete.isPending ? (
                    <Spinner className="size-3.5" aria-hidden="true" />
                  ) : (
                    <IconCheck className="size-3.5" aria-hidden="true" />
                  )}
                  {confirmActionLabel}
                </Button>
              ) : (
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
                  {nextActionLabel}
                </Button>
              )}
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
          ) : completionUserStopped || completionAbnormal ? (
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
          ) : completionLegacy ? (
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
                      onClick={() =>
                        completionAgentConfirmation
                          ? handleCompleteAgent()
                          : handleComplete(false)
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
                      {completionReady
                        ? t("workflowRun.completeNode.disabledHint")
                        : completionNeedsConfirmation
                          ? t("workflowRun.completeNode.unconfirmedHint")
                          : completionUserStopped
                            ? t("workflowRun.completeNode.userStoppedHint")
                            : completionAbnormal
                              ? t("workflowRun.completeNode.abnormalHint")
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
                {openingTurnUnconfirmed
                  ? t("workflowRun.stepCard.forceDescriptionUnconfirmed")
                  : completionUserStopped
                    ? t("workflowRun.stepCard.forceDescriptionUserStopped")
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
                  // The opening turn's own confirmation remains required even on the force
                  // path; only `turn_unconfirmed` needs it (ADR D5).
                  handleComplete(true, openingTurnUnconfirmed);
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
  if (warning === "file_check_unavailable") {
    return translate("workflowRun.stepCard.warning.file_check_unavailable");
  }
  if (warning === "turn_not_settled") {
    return translate("workflowRun.stepCard.warning.turn_not_settled");
  }
  if (warning.startsWith("history_integrity_gap")) {
    return translate("workflowRun.stepCard.warning.history_integrity_gap");
  }
  return warning;
}

/** Human-readable blocker list used by the legacy card and its scoped continue instruction. */
function completionBlockerReasonText(
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

/** Reason copy for each stop cause written by the backend park/prepare path. */
const STOP_CAUSE_REASON_KEYS: Record<WorkflowNodeStopCause, string> = {
  user_stopped: "workflowRun.stepCard.stopCause.user_stopped",
  prompt_timed_out: "workflowRun.stepCard.stopCause.prompt_timed_out",
  stream_dropped: "workflowRun.stepCard.stopCause.stream_dropped",
  session_crashed: "workflowRun.stepCard.stopCause.session_crashed",
  unknown: "workflowRun.stepCard.stopCause.unknown",
};

/** Reason copy for every blocker that can keep a parked turn from completing. */
const ABNORMAL_BLOCKER_REASON_KEYS: Record<
  WorkflowNodeCompletionBlockerKind,
  string
> = {
  turn_stopped: "workflowRun.stepCard.abnormalReason.turn_stopped",
  turn_truncated: "workflowRun.stepCard.abnormalReason.turn_truncated",
  turn_refused: "workflowRun.stepCard.abnormalReason.turn_refused",
  turn_unknown: "workflowRun.stepCard.abnormalReason.turn_unknown",
  turn_unconfirmed: "workflowRun.stepCard.abnormalReason.turn_unconfirmed",
  agent_signal_pending:
    "workflowRun.stepCard.abnormalReason.agent_signal_pending",
  agent_confirmation_pending:
    "workflowRun.stepCard.abnormalReason.agent_confirmation_pending",
  structured_invalid: "workflowRun.stepCard.abnormalReason.structured_invalid",
  output_missing: "workflowRun.stepCard.abnormalReason.output_missing",
  file_missing: "workflowRun.stepCard.abnormalReason.file_missing",
  file_unchanged: "workflowRun.stepCard.abnormalReason.file_unchanged",
};

/**
 * Translation keys naming why a classified park stopped, in display order.
 *
 * The stop cause is authoritative when present, so a blocker that only repeats the stop itself is
 * skipped then; output-contract failures and a mixed-in `turn_unconfirmed` still add their own
 * line. A legacy `awaiting_user` snapshot without a cause is the user's own stop, never a system
 * failure.
 */
function completionParkReasonKeys(
  completion: WorkflowNodeCompletion,
): string[] {
  const hasStopReason =
    completion.stopCause != null || completion.parkKind === "awaiting_user";
  const keys: string[] = [];
  if (completion.stopCause != null) {
    keys.push(STOP_CAUSE_REASON_KEYS[completion.stopCause]);
  } else if (completion.parkKind === "awaiting_user") {
    keys.push(STOP_CAUSE_REASON_KEYS.user_stopped);
  }
  for (const blocker of completion.blockers) {
    if (
      hasStopReason &&
      (blocker.kind === "turn_stopped" ||
        blocker.kind === "turn_truncated" ||
        blocker.kind === "turn_unknown")
    ) {
      continue;
    }
    keys.push(ABNORMAL_BLOCKER_REASON_KEYS[blocker.kind]);
  }
  return [...new Set(keys)];
}

/** Localized stop/contract reasons; equal copy collapses into one line. */
function completionParkReasonTexts(
  completion: WorkflowNodeCompletion,
  translate: (key: string) => string,
): string[] {
  return [
    ...new Set(
      completionParkReasonKeys(completion).map((key) => translate(key)),
    ),
  ];
}
