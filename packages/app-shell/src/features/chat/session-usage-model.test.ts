import type { TokenUsageReport } from "@ora/contracts";
import { describe, expect, it } from "vitest";
import {
  contextUsagePercent,
  shouldShowSessionUsage,
  tokenComposition,
} from "./session-usage-model";

function report(overrides: Partial<TokenUsageReport> = {}): TokenUsageReport {
  return {
    accountingScope: "unspecified",
    totalTokens: 100n,
    inputTokens: 40n,
    outputTokens: 15n,
    thoughtTokens: 10n,
    cachedReadTokens: 30n,
    cachedWriteTokens: 5n,
    ...overrides,
  };
}

describe("session usage presentation model", () => {
  it("keeps the context ring within its visual bounds", () => {
    expect(contextUsagePercent(34, 100)).toBe(34);
    expect(contextUsagePercent(120, 100)).toBe(100);
    expect(contextUsagePercent(10, 0)).toBe(100);
    expect(contextUsagePercent(0, 0)).toBe(0);
  });

  it("only exposes the header entry after usage is reported or unavailable", () => {
    expect(
      shouldShowSessionUsage({
        context: { status: "hidden" },
        lastTurnTokens: { status: "none" },
      }),
    ).toBe(false);
    expect(
      shouldShowSessionUsage({
        context: { status: "needs_interaction" },
        lastTurnTokens: { status: "none" },
      }),
    ).toBe(false);
    expect(
      shouldShowSessionUsage({
        context: { status: "awaiting_report" },
        lastTurnTokens: { status: "awaiting_completion" },
      }),
    ).toBe(false);
    expect(
      shouldShowSessionUsage({
        context: {
          status: "reported",
          snapshot: {
            usedTokens: 34,
            sizeTokens: 100,
            receivedAt: 1,
          },
        },
        lastTurnTokens: { status: "none" },
      }),
    ).toBe(true);
    expect(
      shouldShowSessionUsage({
        context: { status: "unavailable" },
        lastTurnTokens: { status: "none" },
      }),
    ).toBe(true);
    expect(
      shouldShowSessionUsage({
        context: { status: "hidden" },
        lastTurnTokens: {
          status: "reported",
          usage: report(),
          receivedAt: 1,
        },
      }),
    ).toBe(true);
    expect(
      shouldShowSessionUsage({
        context: { status: "hidden" },
        lastTurnTokens: { status: "unavailable" },
      }),
    ).toBe(true);
  });

  it("builds a stack when every reported category adds to the total", () => {
    expect(tokenComposition(report())).toEqual({
      kind: "stacked",
      total: 100n,
      knownSum: 100n,
      segments: [
        { key: "input", value: 40n },
        { key: "output", value: 15n },
        { key: "thought", value: 10n },
        { key: "cachedRead", value: 30n },
        { key: "cachedWrite", value: 5n },
      ],
    });
  });

  it("adds an unclassified segment when reported fields leave a gap", () => {
    expect(
      tokenComposition(
        report({
          thoughtTokens: undefined,
          cachedReadTokens: undefined,
          cachedWriteTokens: undefined,
        }),
      ),
    ).toEqual({
      kind: "stacked",
      total: 100n,
      knownSum: 55n,
      segments: [
        { key: "input", value: 40n },
        { key: "output", value: 15n },
        { key: "unclassified", value: 45n },
      ],
    });
  });

  it("refuses a stack for overlapping fields or a zero total", () => {
    expect(tokenComposition(report({ totalTokens: 90n }))).toEqual({
      kind: "overlap",
      total: 90n,
      knownSum: 100n,
    });
    expect(tokenComposition(report({ totalTokens: 0n }))).toEqual({
      kind: "empty",
      total: 0n,
      knownSum: 100n,
    });
  });
});
