import type { TokenUsageReport } from "@ora/contracts";
import type { SessionUsage } from "@ora/chat";
import { Tooltip, TooltipContent, TooltipTrigger, cn } from "@ora/ui";
import { useTranslation } from "react-i18next";
import {
  asBigInt,
  tokenComposition,
  type TokenSegment,
  type TokenSegmentKey,
} from "./session-usage-model";
import { formatTokenCount, relativeUsageTime } from "./session-usage-format";
import { UsageHeading, UsageMetric } from "./session-usage-primitives";

const SEGMENT_STYLES: Record<TokenSegmentKey, string> = {
  input: "bg-blue-500",
  output: "bg-orange-500",
  thought: "bg-violet-500",
  cachedRead: "bg-emerald-500",
  cachedWrite: "bg-rose-500",
  unclassified: "bg-muted-foreground/40",
};

/** Displays the draft token report bound to the most recently completed response. */
export function TokenSection({
  usage,
  now,
  locale,
}: {
  usage: SessionUsage;
  now: number;
  locale: string;
}) {
  const { t } = useTranslation();
  const state = usage.lastTurnTokens;
  return (
    <section
      aria-labelledby="last-turn-token-usage-title"
      className="space-y-2"
    >
      <UsageHeading
        id="last-turn-token-usage-title"
        title={t("chat.usage.lastTurnTitle")}
        updatedLabel={
          state.status === "reported"
            ? t("chat.usage.updated", {
                time: relativeUsageTime(state.receivedAt, now, t),
              })
            : undefined
        }
      />
      {state.status === "reported" ? (
        <TokenMetrics report={state.usage} locale={locale} />
      ) : (
        <p className="text-xs leading-relaxed text-muted-foreground">
          {t(
            state.status === "awaiting_completion"
              ? "chat.usage.tokenAwaiting"
              : state.status === "unavailable"
                ? "chat.usage.tokenUnavailable"
                : "chat.usage.tokenNeedsInteraction",
          )}
        </p>
      )}
    </section>
  );
}

function TokenMetrics({
  report,
  locale,
}: {
  report: TokenUsageReport;
  locale: string;
}) {
  const { t } = useTranslation();
  const composition = tokenComposition(report);
  const fields: TokenSegment[] = [
    { key: "input", value: asBigInt(report.inputTokens) },
    { key: "output", value: asBigInt(report.outputTokens) },
    ...(report.thoughtTokens === undefined
      ? []
      : [{ key: "thought" as const, value: asBigInt(report.thoughtTokens) }]),
    ...(report.cachedReadTokens === undefined
      ? []
      : [
          {
            key: "cachedRead" as const,
            value: asBigInt(report.cachedReadTokens),
          },
        ]),
    ...(report.cachedWriteTokens === undefined
      ? []
      : [
          {
            key: "cachedWrite" as const,
            value: asBigInt(report.cachedWriteTokens),
          },
        ]),
  ];
  return (
    <>
      <div className="grid grid-cols-3 gap-2 text-xs">
        <UsageMetric
          label={t("chat.usage.total")}
          value={formatTokenCount(asBigInt(report.totalTokens), locale)}
        />
        {fields.map((field) => (
          <UsageMetric
            key={field.key}
            label={segmentLabel(field.key, t)}
            value={formatTokenCount(field.value, locale)}
          />
        ))}
      </div>
      {composition.kind === "stacked" && (
        <>
          <div
            className="flex h-3 overflow-hidden rounded-full bg-muted"
            aria-label={t("chat.usage.composition")}
          >
            {composition.segments
              .filter((segment) => segment.value > 0n)
              .map((segment) => (
                <Tooltip key={segment.key}>
                  <TooltipTrigger
                    render={
                      <button
                        type="button"
                        className={cn(
                          "h-full min-w-0 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
                          SEGMENT_STYLES[segment.key],
                        )}
                        style={{
                          flexGrow: Number(segment.value),
                          flexBasis: 0,
                        }}
                        aria-label={segmentTooltip(
                          segment,
                          composition.total,
                          locale,
                          t,
                        )}
                      />
                    }
                  />
                  <TooltipContent>
                    {segmentTooltip(segment, composition.total, locale, t)}
                  </TooltipContent>
                </Tooltip>
              ))}
          </div>
          <div className="flex flex-wrap gap-x-3 gap-y-1">
            {composition.segments.map((segment) => (
              <span
                key={segment.key}
                className="inline-flex items-center gap-1 text-[11px] text-muted-foreground"
              >
                <span
                  className={cn(
                    "size-2 rounded-sm",
                    SEGMENT_STYLES[segment.key],
                  )}
                  aria-hidden="true"
                />
                {segmentLabel(segment.key, t)}{" "}
                {formatTokenCount(segment.value, locale)}
              </span>
            ))}
          </div>
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            {composition.knownSum === composition.total
              ? t("chat.usage.formulaExact", {
                  total: formatTokenCount(composition.total, locale),
                  parts: composition.segments
                    .map(
                      (segment) =>
                        `${segmentLabel(segment.key, t)} ${formatTokenCount(segment.value, locale)}`,
                    )
                    .join(" + "),
                })
              : t("chat.usage.formulaGap", {
                  total: formatTokenCount(composition.total, locale),
                  known: formatTokenCount(composition.knownSum, locale),
                  gap: formatTokenCount(
                    composition.total - composition.knownSum,
                    locale,
                  ),
                })}
          </p>
        </>
      )}
      {composition.kind === "overlap" && (
        <p className="rounded-md bg-muted/60 p-2 text-[11px] leading-relaxed text-muted-foreground">
          {t("chat.usage.formulaOverlap", {
            known: formatTokenCount(composition.knownSum, locale),
            total: formatTokenCount(composition.total, locale),
          })}
        </p>
      )}
      {composition.kind === "empty" && (
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          {t("chat.usage.zeroTotal")}
        </p>
      )}
    </>
  );
}

function segmentLabel(
  key: TokenSegmentKey,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  return t(`chat.usage.segment.${key}`);
}

function segmentTooltip(
  segment: TokenSegment,
  total: bigint,
  locale: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  const percent = Number((segment.value * 10_000n) / total) / 100;
  return t("chat.usage.segmentTooltip", {
    name: segmentLabel(segment.key, t),
    value: formatTokenCount(segment.value, locale),
    percent: percent.toFixed(1),
  });
}
