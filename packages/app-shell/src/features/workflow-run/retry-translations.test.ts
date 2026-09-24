import { describe, expect, it } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { workflowRunTranslations } from "./translations";

/** Exact copy the waiting-retry and failed-attempt UI depends on, per language. */
const RETRY_STRINGS: Record<string, { "zh-CN": string; "en-US": string }> = {
  "workflowRun.status.retry_waiting": {
    "zh-CN": "等待重试",
    "en-US": "Waiting to retry",
  },
  "workflowRun.retryWait.label": {
    "zh-CN": "等待重试（第 {{attempt}}/{{max}} 次），{{seconds}} 秒后开始",
    "en-US":
      "Waiting to retry (attempt {{attempt}}/{{max}}), starts in {{seconds}}s",
  },
  "workflowRun.retryWait.labelStarting": {
    "zh-CN": "等待重试（第 {{attempt}}/{{max}} 次），即将开始",
    "en-US": "Waiting to retry (attempt {{attempt}}/{{max}}), starting…",
  },
  "workflowRun.retryWait.attempt": {
    "zh-CN": "等待重试（第 {{attempt}}/{{max}} 次）",
    "en-US": "Waiting to retry (attempt {{attempt}}/{{max}})",
  },
  "workflowRun.retryWait.countdown": {
    "zh-CN": "{{seconds}} 秒后开始",
    "en-US": "starts in {{seconds}}s",
  },
  "workflowRun.retryWait.starting": {
    "zh-CN": "即将开始",
    "en-US": "Starting…",
  },
  "workflowRun.retryWait.sessionPending": {
    "zh-CN":
      "上一次尝试失败，Ora 会自动重新运行这个节点；新一次尝试开始后，这里会显示它的会话。",
    "en-US":
      "The last attempt failed and Ora will run this node again by itself; the new attempt's session appears here once it starts.",
  },
  "workflowRun.failedAttempts.title": {
    "zh-CN": "之前失败的尝试",
    "en-US": "Earlier failed attempts",
  },
  "workflowRun.failedAttempts.replacedByRetry": {
    "zh-CN": "已自动重试",
    "en-US": "Retried automatically",
  },
  "workflowRun.failedAttempts.replacedByScheduledRetry": {
    "zh-CN": "已安排自动重试（未开始）",
    "en-US": "Automatic retry scheduled (not started)",
  },
  "workflowRun.failedAttempts.replacedByResume": {
    "zh-CN": "已手动续跑",
    "en-US": "Resumed manually",
  },
  "workflowRun.failedAttempts.beforeRestart": {
    "zh-CN": "从头重新运行前",
    "en-US": "Before “Run again from start”",
  },
  "workflowRun.failedAttempts.rootCause": {
    "zh-CN": "底层原因",
    "en-US": "Underlying error",
  },
  "workflowRun.failedAttempts.chain": {
    "zh-CN": "完整错误链（{{levels}} 层）",
    "en-US": "Full error chain ({{levels}} levels)",
  },
  "workflowRun.retry.exhausted_one": {
    "zh-CN": "已自动重试 {{count}} 次，仍然失败",
    "en-US": "Retried automatically {{count}} time, still failed",
  },
  "workflowRun.retry.exhausted_other": {
    "zh-CN": "已自动重试 {{count}} 次，仍然失败",
    "en-US": "Retried automatically {{count}} times, still failed",
  },
  "workflowRun.retry.notStarted": {
    "zh-CN": "这次自动重试已安排，但没有开始",
    "en-US": "This automatic retry was scheduled but never started",
  },
  "workflowRun.retry.abandoned": {
    "zh-CN": "运行在等待自动重试时结束，这次重试没有开始",
    "en-US":
      "The run ended while this node was waiting to retry, so the retry never started",
  },
  "workflowRun.retry.kindNotRetried": {
    "zh-CN": "这类失败不会自动重试",
    "en-US": "This kind of failure is not retried automatically",
  },
};

const LOCALES = ["zh-CN", "en-US"] as const;

function lookup(locale: (typeof LOCALES)[number], key: string) {
  return (workflowRunTranslations[locale] as Record<string, string>)[key];
}

function placeholders(text: string): string[] {
  return Array.from(text.matchAll(/\{\{(\w+)\}\}/gu), (match) => match[1]!)
    .sort()
    .filter((name, index, all) => all.indexOf(name) === index);
}

describe("retry translations", () => {
  it.each(
    Object.entries(RETRY_STRINGS).flatMap(([key, texts]) =>
      LOCALES.map((locale) => ({ key, locale, text: texts[locale] })),
    ),
  )("$locale $key has the exact copy", ({ key, locale, text }) => {
    expect(lookup(locale, key)).toBe(text);
  });

  it.each(Object.keys(RETRY_STRINGS))(
    "%s uses the same placeholders in both languages",
    (key) => {
      expect(placeholders(lookup("zh-CN", key)!)).toEqual(
        placeholders(lookup("en-US", key)!),
      );
    },
  );

  it.each(LOCALES)(
    "%s has both plural forms of the exhausted note",
    (locale) => {
      const keys = Object.keys(workflowRunTranslations[locale]).filter((key) =>
        key.startsWith("workflowRun.retry.exhausted"),
      );
      expect(keys.sort()).toEqual([
        "workflowRun.retry.exhausted_one",
        "workflowRun.retry.exhausted_other",
      ]);
    },
  );

  it.each([
    { locale: "zh-CN" as const, count: 1, text: "已自动重试 1 次，仍然失败" },
    { locale: "zh-CN" as const, count: 3, text: "已自动重试 3 次，仍然失败" },
    {
      locale: "en-US" as const,
      count: 1,
      text: "Retried automatically 1 time, still failed",
    },
    {
      locale: "en-US" as const,
      count: 3,
      text: "Retried automatically 3 times, still failed",
    },
  ])(
    "$locale resolves the exhausted note for $count",
    ({ locale, count, text }) => {
      expect(
        appI18n.getFixedT(locale)("workflowRun.retry.exhausted", { count }),
      ).toBe(text);
    },
  );
});
