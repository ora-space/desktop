import { useTranslation } from "react-i18next";
import type { Session } from "@ora/contracts";
import { Button } from "@ora/ui";
import { IconAlertTriangle, IconLoader2 } from "@tabler/icons-react";
import { localizeContractError } from "../../i18n/contract-error";
import { useResumeSessionHistory } from "../../state/hooks/use-workspace-mutations";

/**
 * Offers to repair a session whose history stopped being writable.
 *
 * Ora extends a recorded transcript by position, so it refuses to append to a
 * history it could not read or write — which also blocks prompting and
 * switching agent. That leaves the session in a state only an explicit repair
 * can leave, and nothing else in the UI would ever ask for one, so the banner
 * is the entry point rather than a notice.
 *
 * Rendering nothing for a writable session keeps every caller free to mount it
 * unconditionally next to the conversation it belongs to.
 */
export function SessionHistoryBanner({ session }: { session: Session | undefined }) {
  const { t } = useTranslation();
  const resumeHistory = useResumeSessionHistory();

  if (session === undefined || session.historyState.type !== "degraded") return null;

  return (
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
          {session.historyState.reason}
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
  );
}
