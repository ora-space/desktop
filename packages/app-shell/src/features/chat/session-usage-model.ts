import type { TokenUsageReport } from "@ora/contracts";
import type { SessionUsage } from "@ora/chat";

export type TokenSegmentKey =
  | "input"
  | "output"
  | "thought"
  | "cachedRead"
  | "cachedWrite"
  | "unclassified";

export interface TokenSegment {
  key: TokenSegmentKey;
  value: bigint;
}

export type TokenComposition =
  | {
      kind: "stacked";
      total: bigint;
      knownSum: bigint;
      segments: TokenSegment[];
    }
  | { kind: "empty"; total: bigint; knownSum: bigint }
  | { kind: "overlap"; total: bigint; knownSum: bigint };

/** Reports whether the compact context bar should reserve any space for usage. */
export function shouldShowSessionUsage(usage: SessionUsage): boolean {
  return (
    usage.context.status !== "hidden" || usage.lastTurnTokens.status !== "none"
  );
}

/** Clamps the agent's context ratio for presentation while retaining raw counters elsewhere. */
export function contextUsagePercent(used: number, size: number): number {
  if (size <= 0) return used > 0 ? 100 : 0;
  return Math.min(Math.max((used / size) * 100, 0), 100);
}

/** Classifies whether the agent's token fields form a reliable stacked composition. */
export function tokenComposition(report: TokenUsageReport): TokenComposition {
  const total = asBigInt(report.totalTokens);
  const segments: TokenSegment[] = [
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
  const knownSum = segments.reduce((sum, segment) => sum + segment.value, 0n);
  if (total === 0n) return { kind: "empty", total, knownSum };
  if (knownSum > total) return { kind: "overlap", total, knownSum };
  if (knownSum < total) {
    segments.push({ key: "unclassified", value: total - knownSum });
  }
  return { kind: "stacked", total, knownSum, segments };
}

/** Accepts JSON number values defensively even though generated u64 contracts use bigint. */
export function asBigInt(value: bigint | number): bigint {
  return typeof value === "bigint" ? value : BigInt(Math.trunc(value));
}
