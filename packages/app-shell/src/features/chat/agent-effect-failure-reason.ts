import type { TFunction } from "i18next";
import type {
  AgentEffectFailureReason,
  AgentEffectReadiness,
} from "../../state/hooks/use-agent-effect-readiness";

/** Reads one non-empty safe Condition parameter without trimming meaningful characters. */
function nonEmptyParameter(
  parameters: Record<string, string> | undefined,
  key: string,
): string | null {
  const value = parameters?.[key];
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

/**
 * Formats one persisted Effect blocking Condition in the active UI language.
 *
 * Known codes with required structured parameters are localized from `code` and
 * `parameters` without parsing backend English text; unknown codes or legacy
 * rows without parameters fall back to the backend-approved safe message.
 */
export function localizeAgentEffectFailureReason(
  reason: AgentEffectFailureReason,
  t: TFunction,
): string {
  const relativePath = nonEmptyParameter(reason.parameters, "relativePath");
  switch (reason.code) {
    case "preserved_item_conflict":
      return relativePath !== null
        ? t("chat.agentEffectFailure.reason.preserved_item_conflict", {
            relativePath,
          })
        : reason.message;
    case "skill_directory_link":
      return relativePath !== null
        ? t("chat.agentEffectFailure.reason.skill_directory_link", {
            relativePath,
          })
        : reason.message;
    case "unsafe_skill_directory":
      return relativePath !== null
        ? t("chat.agentEffectFailure.reason.unsafe_skill_directory", {
            relativePath,
          })
        : reason.message;
    case "managed_item_drift":
      return relativePath !== null
        ? t("chat.agentEffectFailure.reason.managed_item_drift", {
            relativePath,
          })
        : t("chat.agentEffectFailure.reason.managed_item_drift_generic");
    case "recovery_required":
      return t("chat.agentEffectFailure.reason.recovery_required");
    default:
      return reason.message;
  }
}

/** Localizes the primary failure reason for compact surfaces such as the send-button hint. */
export function localizeAgentEffectFailureSummary(
  readiness: Extract<AgentEffectReadiness, { kind: "failed" }>,
  t: TFunction,
): string {
  const primary = readiness.reasons.find(
    (reason) => reason.message === readiness.message,
  ) ??
    readiness.reasons[0] ?? {
      code: readiness.recoveryRequired ? "recovery_required" : "effect_failure",
      message: readiness.message,
      parameters: {},
    };
  return localizeAgentEffectFailureReason(primary, t);
}
