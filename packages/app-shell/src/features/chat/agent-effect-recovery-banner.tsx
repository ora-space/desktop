import { useTranslation } from "react-i18next";
import { IconRefresh } from "@tabler/icons-react";
import { useRecoverEffectTarget } from "../../state/hooks/use-recover-effect-target";
import { useRetryEffectTarget } from "../../state/hooks/use-retry-effect-target";
import { localizeContractError } from "../../i18n/contract-error";
import type { AgentEffectReadiness } from "../../state/hooks/use-agent-effect-readiness";

/**
 * Explains one failed Agent Effect Target and offers its owned recovery action.
 *
 * Recovery closes quarantined journals; every other visible failure is retried after the
 * user fixes the external cause. Keeping both actions here means callers can mount this
 * beside the conversation without reconstructing which command belongs to the state.
 */
export function AgentEffectRecoveryBanner({
  readiness,
}: {
  readiness: AgentEffectReadiness;
}) {
  const { t } = useTranslation();
  const recovery = useRecoverEffectTarget();
  const retry = useRetryEffectTarget();

  if (readiness.kind !== "failed" || readiness.targetId === null) return null;
  const action = readiness.recoveryRequired ? recovery : retry;
  // The condition message is backend-approved safe detail (SafeConditionDetails), not a
  // raw Error.message, so rendering it verbatim is the same contract the send hint uses.
  const reasons =
    readiness.reasons.length > 0
      ? readiness.reasons
      : [{ code: "effect_failure", message: readiness.message }];
  const reasonMessages = reasons.map((reason) => reason.message);
  const hasPreservedConflict = reasons.some(
    (reason) => reason.code === "preserved_item_conflict",
  );
  const title = readiness.recoveryRequired
    ? t("chat.agentEffectRecovery.title")
    : t("chat.agentEffectFailure.title");
  const hint = readiness.recoveryRequired
    ? null
    : t(
        hasPreservedConflict
          ? "chat.agentEffectFailure.preservedHint"
          : "chat.agentEffectFailure.retryHint",
      );
  const targetId = readiness.targetId;
  return (
    <div
      role="alert"
      // Names the action state without depending on the localized sentence, so tests
      // and bug reports can tell a pending action apart from a refused one.
      data-effect-failure={
        action.isPending ? "pending" : action.isError ? "failed" : "required"
      }
      data-effect-action={readiness.recoveryRequired ? "recover" : "retry"}
      className="mx-3 mb-2 flex items-start gap-2 rounded-md border border-red-500/30 bg-red-500/5 px-3 py-2 text-xs sm:mx-4"
    >
      <IconRefresh
        className="mt-0.5 size-4 shrink-0 text-red-600 dark:text-red-400"
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1">
        <p className="font-medium text-red-700 dark:text-red-300">{title}</p>
        <ul className="mt-0.5 max-h-40 space-y-0.5 overflow-y-auto break-words text-muted-foreground">
          {reasonMessages.map((message, index) => (
            <li key={`${index}:${message}`}>{message}</li>
          ))}
        </ul>
        {hint !== null && (
          <p className="mt-1 break-words text-muted-foreground">{hint}</p>
        )}
        {action.isError && (
          <p className="mt-0.5 break-words text-red-700 dark:text-red-300">
            {t(
              readiness.recoveryRequired
                ? "chat.agentEffectRecovery.refused"
                : "chat.agentEffectFailure.refused",
              {
                message: localizeContractError(action.error, t),
              },
            )}
          </p>
        )}
      </div>
      <button
        type="button"
        disabled={action.isPending}
        onClick={() => action.mutate(targetId)}
        className="shrink-0 rounded-md border border-red-500/40 px-2.5 py-1 font-medium text-red-700 transition-colors hover:bg-red-500/10 disabled:opacity-50 dark:text-red-300"
      >
        {readiness.recoveryRequired
          ? recovery.isPending
            ? t("chat.agentEffectRecovery.pending")
            : t("chat.agentEffectRecovery.action")
          : retry.isPending
            ? t("chat.agentEffectFailure.pending")
            : t("chat.agentEffectFailure.action")}
      </button>
    </div>
  );
}
