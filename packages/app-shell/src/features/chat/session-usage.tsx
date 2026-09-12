import { useState } from "react";
import { IconHelpCircle } from "@tabler/icons-react";
import type { ContextUsageSnapshot, SessionUsage } from "@ora/chat";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
} from "@ora/ui";
import { useTranslation } from "react-i18next";
import { contextUsagePercent } from "./session-usage-model";
import {
  formatCompactTokens,
  relativeUsageTime,
  useMinuteClock,
} from "./session-usage-format";
import { UsageHeading, UsageMetric } from "./session-usage-primitives";
import { TokenSection } from "./session-token-usage";

/** Displays volatile context and last-turn token telemetry above the composer. */
export function SessionUsageIndicator({ usage }: { usage: SessionUsage }) {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(false);
  const now = useMinuteClock();
  const triggerLabel = contextTriggerLabel(usage, i18n.language, t);

  return (
    <div className="flex min-w-0 flex-1 items-center justify-end gap-0.5">
      <Popover open={open} onOpenChange={setOpen}>
        <Tooltip>
          <TooltipTrigger
            render={
              <PopoverTrigger
                render={
                  <button
                    type="button"
                    className="flex h-6 items-center gap-1.5 rounded-md px-1.5 text-xs text-muted-foreground outline-none transition-colors hover:bg-muted/60 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                    aria-label={triggerLabel}
                  />
                }
              />
            }
          >
            <UsageTrigger usage={usage} now={now} />
          </TooltipTrigger>
          <TooltipContent>{triggerLabel}</TooltipContent>
        </Tooltip>
        <PopoverContent side="top" align="end" className="w-[26rem] gap-3 p-3">
          <ContextSection usage={usage} now={now} locale={i18n.language} />
          <div className="h-px bg-border" />
          <TokenSection usage={usage} now={now} locale={i18n.language} />
          <p className="border-t border-border pt-2 text-[11px] leading-relaxed text-muted-foreground">
            {t("chat.usage.disclaimer")}
          </p>
        </PopoverContent>
      </Popover>
      <Tooltip>
        <TooltipTrigger
          render={
            <button
              type="button"
              className="flex size-6 items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-muted/60 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
              aria-label={t("chat.usage.referenceInfo")}
              onClick={() => setOpen(true)}
            />
          }
        >
          <IconHelpCircle className="size-3.5" />
        </TooltipTrigger>
        <TooltipContent>{t("chat.usage.disclaimer")}</TooltipContent>
      </Tooltip>
    </div>
  );
}

function UsageTrigger({ usage, now }: { usage: SessionUsage; now: number }) {
  const { t } = useTranslation();
  if (usage.context.status === "reported") {
    const { usedTokens, sizeTokens } = usage.context.snapshot;
    const percent = contextUsagePercent(usedTokens, sizeTokens);
    return (
      <>
        <span
          className={cn(
            "relative size-[18px] shrink-0 rounded-full",
            percent >= 95
              ? "text-destructive"
              : percent >= 75
                ? "text-amber-500"
                : "text-primary",
          )}
          style={{
            background: `conic-gradient(currentColor ${percent}%, var(--muted) 0)`,
          }}
          aria-hidden="true"
        >
          <span className="absolute inset-[3px] rounded-full bg-background" />
        </span>
        <span className="text-[11px]">
          {t("chat.usage.updated", {
            time: relativeUsageTime(usage.context.snapshot.receivedAt, now, t),
          })}
        </span>
      </>
    );
  }
  return (
    <>
      <span>
        {t(
          usage.context.status === "needs_interaction"
            ? "chat.usage.waiting"
            : usage.context.status === "awaiting_report"
              ? "chat.usage.awaiting"
              : "chat.usage.unavailable",
        )}
      </span>
      <IconHelpCircle className="size-3.5" aria-hidden="true" />
    </>
  );
}

function ContextSection({
  usage,
  now,
  locale,
}: {
  usage: SessionUsage;
  now: number;
  locale: string;
}) {
  const { t } = useTranslation();
  const context = usage.context;
  return (
    <section
      aria-labelledby="session-context-usage-title"
      className="space-y-2"
    >
      <UsageHeading
        id="session-context-usage-title"
        title={t("chat.usage.contextTitle")}
        tooltip={t("chat.usage.contextTooltip")}
        details={t("chat.usage.contextDetails")}
      />
      {context.status === "reported" ? (
        <ContextMetrics snapshot={context.snapshot} now={now} locale={locale} />
      ) : (
        <p className="text-xs leading-relaxed text-muted-foreground">
          {t(
            context.status === "needs_interaction"
              ? "chat.usage.reloadEmpty"
              : context.status === "awaiting_report"
                ? "chat.usage.awaitingDetails"
                : "chat.usage.contextUnavailable",
          )}
        </p>
      )}
    </section>
  );
}

function ContextMetrics({
  snapshot,
  now,
  locale,
}: {
  snapshot: ContextUsageSnapshot;
  now: number;
  locale: string;
}) {
  const { t } = useTranslation();
  const percent = contextUsagePercent(snapshot.usedTokens, snapshot.sizeTokens);
  const remaining = Math.max(snapshot.sizeTokens - snapshot.usedTokens, 0);
  return (
    <>
      <div className="grid grid-cols-2 gap-2 text-xs">
        <UsageMetric
          label={t("chat.usage.used")}
          value={formatCompactTokens(snapshot.usedTokens, locale)}
        />
        <UsageMetric
          label={t("chat.usage.limit")}
          value={formatCompactTokens(snapshot.sizeTokens, locale)}
        />
        <UsageMetric
          label={t("chat.usage.remaining")}
          value={formatCompactTokens(remaining, locale)}
        />
        <UsageMetric
          label={t("chat.usage.percent")}
          value={`${Math.round(percent)}%`}
        />
        {snapshot.cost && (
          <UsageMetric
            label={t("chat.usage.sessionCost")}
            value={formatCost(snapshot.cost, locale)}
          />
        )}
      </div>
      <p className="text-[11px] text-muted-foreground">
        {t("chat.usage.updated", {
          time: relativeUsageTime(snapshot.receivedAt, now, t),
        })}
      </p>
    </>
  );
}

function contextTriggerLabel(
  usage: SessionUsage,
  locale: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (usage.context.status !== "reported") {
    return t(
      usage.context.status === "needs_interaction"
        ? "chat.usage.reloadEmpty"
        : usage.context.status === "awaiting_report"
          ? "chat.usage.awaitingDetails"
          : "chat.usage.contextUnavailable",
    );
  }
  const snapshot = usage.context.snapshot;
  return t("chat.usage.contextSummary", {
    percent: Math.round(
      contextUsagePercent(snapshot.usedTokens, snapshot.sizeTokens),
    ),
    used: formatCompactTokens(snapshot.usedTokens, locale),
    size: formatCompactTokens(snapshot.sizeTokens, locale),
  });
}

function formatCost(
  cost: { amount: number; currency: string },
  locale: string,
): string {
  try {
    return new Intl.NumberFormat(locale, {
      style: "currency",
      currency: cost.currency,
    }).format(cost.amount);
  } catch {
    return `${cost.amount} ${cost.currency}`;
  }
}
