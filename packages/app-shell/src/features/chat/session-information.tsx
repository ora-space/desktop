import { useEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { groupSubagentItems } from "./subagent-items";
import { locateSessionChild } from "./session-child-navigation";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import {
  IconAlertTriangle,
  IconCheck,
  IconClock,
  IconCopy,
  IconRefresh,
} from "@tabler/icons-react";
import {
  Badge,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  Separator,
} from "@ora/ui";
import { useChatStore } from "../../chat-store-context";
import { useSessionBindings } from "../../state/data/session-bindings";
import type { SessionBindings } from "@ora/contracts";
import { RunDiagnosticsPanel } from "./run-diagnostics";

/** One panel shared by ordinary conversations and every workflow execution reference. */
export function SessionInformation({ sessionId }: { sessionId?: string }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="ghost" size="sm" onClick={() => setOpen(true)}>
        {t("chat.sessionInformation")}
      </Button>
      {open && (
        <SessionInformationDialog
          key={sessionId ?? "new"}
          sessionId={sessionId}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}

/** Unmounting a dialog tears down its scoped query and prevents late adoption in another session. */
export function SessionInformationDialog({
  sessionId,
  onClose,
}: {
  sessionId?: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const navigating = useRef(false);
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent
        finalFocus={() => (navigating.current ? false : undefined)}
        className="max-h-[85vh] overflow-y-auto sm:max-w-2xl"
      >
        <DialogHeader>
          <DialogTitle>{t("chat.sessionInformation")}</DialogTitle>
          <DialogDescription className="sr-only">
            {t("chat.sessionInformationDescription")}
          </DialogDescription>
        </DialogHeader>
        {sessionId === undefined ? (
          <p className="text-xs text-muted-foreground">
            {t("chat.bindingNotCreated")}
          </p>
        ) : (
          <SessionInformationBody
            key={sessionId}
            sessionId={sessionId}
            onClose={() => {
              navigating.current = true;
              onClose();
            }}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

/** Bordered section card with a tinted header row and an optional header action. */
function Section({
  title,
  action,
  children,
}: {
  title: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="overflow-hidden rounded-lg border">
      <header className="flex items-center justify-between gap-2 border-b bg-muted/40 px-3 py-2">
        <h3 className="text-xs font-medium">{title}</h3>
        {action}
      </header>
      <div className="space-y-2 p-3">{children}</div>
    </section>
  );
}

/** Reads only when explicitly opened; live observations remain owned by the chat store. */
function SessionInformationBody({
  sessionId,
  onClose,
}: {
  sessionId: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const query = useSessionBindings(sessionId);
  const chatStore = useChatStore();
  const conversation = useStore(
    chatStore,
    (state) => state.conversations[sessionId],
  );
  const [locationMissing, setLocationMissing] = useState(false);
  const childButtons = conversation?.turns.flatMap((turn) =>
    groupSubagentItems(turn.items).flatMap((item) =>
      item.kind === "subagentGroup"
        ? [
            <Button
              key={`${turn.id}:${item.id}`}
              variant="outline"
              size="sm"
              onClick={() => {
                if (
                  !locateSessionChild(
                    sessionId,
                    `${turn.id}:display:${item.id}`,
                  )
                ) {
                  setLocationMissing(true);
                  return;
                }
                flushSync(onClose);
              }}
            >
              {t("chat.bindingLocateChildren")} · {item.title ?? item.sourceId}
            </Button>,
          ]
        : [],
    ),
  );
  return (
    <div className="space-y-4 text-xs">
      <Section title="Ora Session ID">
        <CopyIdentifier value={sessionId} />
      </Section>
      <Section
        title={t("chat.bindingTimeline")}
        action={
          <Button
            variant="ghost"
            size="sm"
            className="px-1.5 text-muted-foreground"
            title={t("chat.bindingRefresh")}
            aria-label={t("chat.bindingRefresh")}
            onClick={() => void query.refetch()}
          >
            <IconRefresh size={14} aria-hidden />
          </Button>
        }
      >
        {query.isPending ? (
          <p className="text-muted-foreground">{t("chat.bindingLoading")}</p>
        ) : query.isError ? (
          <p role="alert" className="text-destructive">
            {t("chat.bindingUnavailable")}
          </p>
        ) : (
          <BindingTimeline result={query.data} />
        )}
      </Section>
      <RunDiagnosticsPanel
        diagnostics={conversation?.diagnostics}
        turns={conversation?.turns ?? []}
        isResponding={conversation?.isResponding ?? false}
        pendingPermissions={conversation?.pendingPermissions ?? []}
      />
      <Separator />
      <p className="text-muted-foreground">{t("chat.bindingChildLocation")}</p>
      {childButtons !== undefined && childButtons.length > 0 && (
        <div className="flex flex-wrap gap-2">{childButtons}</div>
      )}
      {locationMissing && (
        <p role="status" className="text-destructive">
          {t("chat.bindingChildrenUnavailable")}
        </p>
      )}
    </div>
  );
}

/** Renders verified segments only; unavailable histories never masquerade as never-created. */
export function BindingTimeline({ result }: { result?: SessionBindings }) {
  const { t } = useTranslation();
  if (result?.type !== "available")
    return (
      <p className="text-muted-foreground">
        {t(
          result?.type === "retry"
            ? "chat.bindingRetry"
            : result?.type === "not_found"
              ? "chat.bindingNotFound"
              : "chat.bindingUnavailable",
        )}
      </p>
    );
  return (
    <>
      {result.history !== "readable" && (
        <p role="status" className="text-muted-foreground">
          {t(`chat.bindingHistory_${result.history}`)}
        </p>
      )}
      {result.segments.length === 0 && result.history === "readable" && (
        <p className="text-muted-foreground">{t("chat.bindingNotCreated")}</p>
      )}
      <ol className="space-y-2">
        {result.segments.map((segment) => (
          <li
            key={segment.key}
            className="space-y-1.5 rounded-md border bg-muted/30 p-2.5"
          >
            <p className="flex flex-wrap items-center gap-x-1.5 gap-y-1 font-medium">
              <span className="break-all">{segment.agentRef}</span>
              {segment.current && (
                <Badge variant="secondary">{t("chat.bindingCurrent")}</Badge>
              )}
            </p>
            <CopyIdentifier value={segment.agentSessionId} />
            <p className="flex items-center gap-1 text-muted-foreground">
              <IconClock size={12} aria-hidden />
              {segment.recordedAt === null ? (
                t(
                  segment.key === "live"
                    ? "chat.bindingUnrecorded"
                    : "chat.bindingTimeUnknown",
                )
              ) : (
                <>
                  {t("chat.bindingRecordedAt")}{" "}
                  <time dateTime={segment.recordedAt}>
                    {new Date(segment.recordedAt).toLocaleString()}
                  </time>
                </>
              )}
            </p>
          </li>
        ))}
      </ol>
    </>
  );
}

/**
 * Copies the entire opaque identifier. Every feedback state occupies the same grid cell, so the
 * button keeps the widest label's box and the identifier never reflows when feedback appears.
 */
function CopyIdentifier({ value }: { value: string }) {
  const { t } = useTranslation();
  const [feedback, setFeedback] = useState<
    { kind: "copied" | "copyFailed" } | undefined
  >();
  useEffect(() => {
    if (!feedback) return;
    const timer = setTimeout(() => setFeedback(undefined), 1600);
    return () => clearTimeout(timer);
  }, [feedback]);
  const states = [
    { kind: undefined, label: t("chat.bindingCopy") },
    { kind: "copied" as const, label: t("chat.binding_copied") },
    { kind: "copyFailed" as const, label: t("chat.binding_copyFailed") },
  ];
  return (
    <div className="flex items-center gap-2">
      <code className="min-w-0 flex-1 select-text break-all rounded-md border bg-muted/50 px-2 py-1.5 font-mono text-[11px] leading-relaxed">
        {value}
      </code>
      <Button
        variant="outline"
        size="sm"
        className="shrink-0"
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(value);
            setFeedback({ kind: "copied" });
          } catch {
            setFeedback({ kind: "copyFailed" });
          }
        }}
      >
        <span className="grid">
          {states.map((state) => {
            const active = feedback?.kind === state.kind;
            return (
              <span
                key={state.kind ?? "idle"}
                aria-hidden={!active}
                className={`col-start-1 row-start-1 inline-flex items-center gap-1 ${
                  active ? "" : "invisible"
                }`}
              >
                {state.kind === "copied" ? (
                  <IconCheck
                    size={14}
                    className="text-emerald-600"
                    aria-hidden
                  />
                ) : state.kind === "copyFailed" ? (
                  <IconAlertTriangle
                    size={14}
                    className="text-destructive"
                    aria-hidden
                  />
                ) : (
                  <IconCopy size={14} aria-hidden />
                )}
                {state.label}
              </span>
            );
          })}
        </span>
      </Button>
      <span role="status" className="sr-only">
        {feedback ? t(`chat.binding_${feedback.kind}`) : ""}
      </span>
    </div>
  );
}
