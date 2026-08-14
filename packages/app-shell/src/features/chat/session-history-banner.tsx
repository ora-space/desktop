import { useTranslation } from "react-i18next";
import type { Session, SessionHistoryNotice } from "@ora/contracts";
import { Button } from "@ora/ui";
import { IconAlertTriangle, IconLoader2 } from "@tabler/icons-react";
import { localizeContractError } from "../../i18n/contract-error";
import { useResumeSessionHistory } from "../../state/hooks/use-workspace-mutations";

/**
 * Surfaces durable-history gaps and offers repair when recording stopped.
 *
 * Damage discovered while reading old records is only a warning: its original
 * position is unknowable, but it does not imply that current writes are broken.
 * A live write failure is different; Ora refuses later prompts until the user
 * explicitly resumes recording, so that banner also provides the repair action.
 *
 * Rendering nothing when neither condition exists keeps callers free to mount
 * the component unconditionally next to the conversation it belongs to.
 */
export function SessionHistoryBanner({
  session,
  notices,
}: {
  session: Session | undefined;
  notices: SessionHistoryNotice[];
}) {
  const { t } = useTranslation();
  const resumeHistory = useResumeSessionHistory();
  const degradedReason = session?.historyState.type === "degraded"
    ? session.historyState.reason
    : null;

  if (degradedReason === null && notices.length === 0) return null;

  return (
    <>
      {notices.length > 0 && (
        <div
          role="alert"
          className="mx-3 mb-2 flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs sm:mx-4"
        >
          <IconAlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <p className="font-medium text-amber-700 dark:text-amber-300">
              {t("chat.historyNotice.title")}
            </p>
            {notices.map((notice, index) => (
              <p key={`${notice.type}-${index}`} className="mt-0.5 break-words text-muted-foreground">
                {notice.type === "unreadable_records"
                  ? t("chat.historyNotice.unreadableRecords", { count: notice.count })
                  : t("chat.historyNotice.unrecordedContent", { reason: notice.reason })}
              </p>
            ))}
          </div>
        </div>
      )}
      {degradedReason !== null && session !== undefined && (
        <div
          role="alert"
          className="mx-3 mb-2 flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs sm:mx-4"
        >
          <IconAlertTriangle className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <p className="font-medium text-destructive">{t("chat.historyDegraded.title")}</p>
            {/* The backend's reason is the only description of what actually broke
                — a full disk reads very differently from a missing file — so it is
                shown verbatim rather than flattened into one generic sentence. */}
            <p className="mt-0.5 break-words text-muted-foreground">
              {degradedReason}
            </p>
            {resumeHistory.isError && (
              <p className="mt-1 text-destructive">
                {localizeContractError(resumeHistory.error, t)}
              </p>
            )}
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-7 shrink-0 text-xs"
            disabled={resumeHistory.isPending}
            onClick={() => resumeHistory.mutate({ sessionId: session.id })}
          >
            {resumeHistory.isPending && (
              <IconLoader2 className="size-3 animate-spin" aria-hidden="true" />
            )}
            {t("chat.historyDegraded.resume")}
          </Button>
        </div>
      )}
    </>
  );
}
