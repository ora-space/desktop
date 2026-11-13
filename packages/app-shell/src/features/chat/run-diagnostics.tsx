import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { IconChevronRight } from "@tabler/icons-react";
import type { ChatTurn, RunDiagnostics } from "@ora/chat";
import type { SessionPermissionRequest } from "@ora/contracts";

/** Shows observations separately from parent progress; silence is never rendered as health. */
export function RunDiagnosticsPanel({
  diagnostics,
  turns,
  isResponding,
  pendingPermissions,
}: {
  diagnostics?: RunDiagnostics;
  turns: ChatTurn[];
  isResponding: boolean;
  pendingPermissions: SessionPermissionRequest[];
}) {
  const { t } = useTranslation();
  const [now, setNow] = useState(Date.now);
  const lastActivity = diagnostics?.lastActivity;
  useEffect(() => {
    if (!isResponding || lastActivity === undefined) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [isResponding, lastActivity]);
  const tools = isResponding
    ? (turns
        .at(-1)
        ?.items.filter(
          (item) =>
            item.kind === "toolCall" &&
            (item.status === "pending" || item.status === "in_progress"),
        ) ?? [])
    : [];
  const provider = diagnostics?.provider;
  const parentTools = tools.filter(
    (tool) => diagnostics?.toolSources?.[tool.id] === "parent",
  );
  const childTools = tools.filter(
    (tool) => diagnostics?.toolSources?.[tool.id] === "child",
  );
  const unknownTools = tools.filter(
    (tool) => diagnostics?.toolSources?.[tool.id] === undefined,
  );
  const childActivity = diagnostics?.lastChildActivity;
  const report =
    provider?.kind === "retry"
      ? t("chat.diagnosticsRetry", { attempt: provider.attempt })
      : provider?.kind === "compaction"
        ? t(`chat.diagnosticsCompaction_${provider.status}`)
        : t("chat.diagnosticsUnknown");
  return (
    <details
      className="group overflow-hidden rounded-lg border text-xs"
      data-testid="run-diagnostics"
    >
      <summary className="flex cursor-pointer list-none items-center gap-1.5 px-3 py-2 font-medium text-muted-foreground hover:bg-muted/50 [&::-webkit-details-marker]:hidden">
        <IconChevronRight
          size={14}
          className="transition-transform group-open:rotate-90"
          aria-hidden
        />
        {t("chat.diagnosticsTitle")}
      </summary>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 border-t px-3 py-2.5">
        <dt className="text-muted-foreground">{t("chat.diagnosticsParent")}</dt>
        <dd>
          {isResponding
            ? t("chat.diagnosticsActive")
            : t("chat.diagnosticsInactive")}
        </dd>
        <dt className="text-muted-foreground">
          {t("chat.diagnosticsProvider")}
        </dt>
        <dd>{report}</dd>
        <dt className="text-muted-foreground">
          {t("chat.diagnosticsLastActivity")}
        </dt>
        <dd>
          {lastActivity === undefined ? (
            diagnostics === undefined ? (
              // No live snapshot: the transcript came from replay, so silence here
              // is an unknown, not a statement that the attempt produced nothing.
              t("chat.diagnosticsNoLiveTime")
            ) : isResponding ? (
              t("chat.diagnosticsNoActivityYet")
            ) : (
              t("chat.diagnosticsNoActivity")
            )
          ) : (
            <>
              {t(`chat.diagnosticsActivity_${lastActivity.kind}`)} ·{" "}
              <time dateTime={new Date(lastActivity.receivedAt).toISOString()}>
                {new Date(lastActivity.receivedAt).toLocaleString()}
              </time>
              {isResponding && (
                <>
                  {" "}
                  ·{" "}
                  {t("chat.diagnosticsSilence", {
                    seconds: Math.max(
                      0,
                      Math.floor((now - lastActivity.receivedAt) / 1000),
                    ),
                  })}
                </>
              )}
            </>
          )}
        </dd>
        {childActivity && (
          <>
            <dt className="text-muted-foreground">
              {t("chat.diagnosticsChildActivity")}
            </dt>
            <dd>
              {t(`chat.diagnosticsActivity_${childActivity.kind}`)} ·{" "}
              <time dateTime={new Date(childActivity.receivedAt).toISOString()}>
                {new Date(childActivity.receivedAt).toLocaleString()}
              </time>
            </dd>
          </>
        )}
        <dt className="text-muted-foreground">
          {t("chat.diagnosticsRequests")}
        </dt>
        <dd>
          {pendingPermissions.length
            ? t("chat.diagnosticsWaiting", { count: pendingPermissions.length })
            : t("chat.diagnosticsNoRequests")}
        </dd>
        <dt className="text-muted-foreground">{t("chat.diagnosticsTools")}</dt>
        <dd>
          {parentTools.length
            ? parentTools
                .map((tool) => (tool.kind === "toolCall" ? tool.title : ""))
                .join(" · ")
            : t("chat.diagnosticsNoTools")}
        </dd>
        {childTools.length > 0 && (
          <>
            <dt className="text-muted-foreground">
              {t("chat.diagnosticsChildTools")}
            </dt>
            <dd>
              {childTools
                .map((tool) => (tool.kind === "toolCall" ? tool.title : ""))
                .join(" · ")}
            </dd>
          </>
        )}
        {unknownTools.length > 0 && (
          <>
            <dt className="text-muted-foreground">
              {t("chat.diagnosticsUnknownTools")}
            </dt>
            <dd>
              {unknownTools
                .map((tool) => (tool.kind === "toolCall" ? tool.title : ""))
                .join(" · ")}
            </dd>
          </>
        )}
      </dl>
      <div className="space-y-1 border-t bg-muted/30 px-3 py-2">
        <p className="text-muted-foreground">{t("chat.diagnosticsLimit")}</p>
        <p className="text-muted-foreground">
          {t("chat.diagnosticsRequestLimit")}
        </p>
      </div>
    </details>
  );
}
