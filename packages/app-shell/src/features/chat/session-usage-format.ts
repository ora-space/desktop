import { useEffect, useState } from "react";
import type { useTranslation } from "react-i18next";

type Translate = ReturnType<typeof useTranslation>["t"];

/** Updates relative timestamps at minute granularity while the popover is mounted. */
export function useMinuteClock(): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

/** Formats one context counter compactly for the 24px composer chrome. */
export function formatCompactTokens(value: number, locale: string): string {
  return new Intl.NumberFormat(locale, {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);
}

/** Formats one exact token counter without losing bigint precision. */
export function formatTokenCount(value: bigint, locale: string): string {
  return new Intl.NumberFormat(locale).format(value);
}

/** Produces the localized age of one in-memory report. */
export function relativeUsageTime(
  receivedAt: number,
  now: number,
  t: Translate,
): string {
  const minutes = Math.max(Math.floor((now - receivedAt) / 60_000), 0);
  return minutes < 1
    ? t("chat.usage.justNow")
    : t("chat.usage.minutesAgo", { count: minutes });
}
