import { describe, expect, it } from "vitest";
import { workflowRunTranslations } from "./translations";

const LOCALES = ["zh-CN", "en-US"] as const;

const STEP_CARD_COPY = {
  "workflowRun.stepCard.pausedBadge": {
    "zh-CN": "已暂停",
    "en-US": "Paused",
  },
  "workflowRun.stepCard.waitingBadge": {
    "zh-CN": "等待参与",
    "en-US": "Awaiting input",
  },
  "workflowRun.stepCard.readyAction": {
    "zh-CN": "完成并进入下一步：{{next}}",
    "en-US": "Complete and continue to: {{next}}",
  },
  "workflowRun.stepCard.readyLastAction": {
    "zh-CN": "完成当前步骤",
    "en-US": "Complete current step",
  },
  "workflowRun.stepCard.continueChat": {
    "zh-CN": "继续对话",
    "en-US": "Continue chatting",
  },
  "workflowRun.stepCard.continueStep": {
    "zh-CN": "继续当前步骤",
    "en-US": "Continue this step",
  },
  "workflowRun.stepCard.forceAction": {
    "zh-CN": "仍然完成（采用当前产出）",
    "en-US": "Complete anyway (use current output)",
  },
  "workflowRun.stepCard.forceConfirm": {
    "zh-CN": "仍然完成",
    "en-US": "Complete anyway",
  },
  "workflowRun.stepCard.provisional": {
    "zh-CN": "本轮仍在收尾；点完成时会按本轮重新校验，未结束会给出原因。",
    "en-US":
      "This turn is still settling; completing re-checks it and reports a reason if it has not finished.",
  },
  "workflowRun.stepCard.unconfirmedTitle": {
    "zh-CN": "等待你的确认",
    "en-US": "Waiting for your confirmation",
  },
  "workflowRun.stepCard.unconfirmedHint": {
    "zh-CN":
      "本次停靠来自 workflow 的首轮自动回合，不代表本步骤已完成。如果智能体在等你回答，请直接在下方回复；也可以继续当前步骤，或显式采用当前产出。",
    "en-US":
      "This park came from the workflow's opening turn, so the step is not complete yet. If the agent is waiting for your answer, reply below; you can also continue the current step or explicitly accept the current output.",
  },
  "workflowRun.stepCard.forceDescriptionUnconfirmed": {
    "zh-CN":
      "本轮已正常结束，但还没有你的确认。完成后会采用当前产出并进入下一步，并记录这次确认。",
    "en-US":
      "The turn ended normally, but you have not confirmed this step. Completing it adopts the current output, advances the workflow, and records this confirmation.",
  },
  "workflowRun.stepCard.continueUnconfirmedPrompt": {
    "zh-CN":
      "请继续当前步骤。本轮自动回合已经结束，但还没有我的确认；如果还需要我提供信息，请直接问我。只完成本步骤，不要执行后续节点；完成后请告诉我结果，我会在工作流界面确认完成。",
    "en-US":
      "Please continue the current step. The opening turn has ended, but I have not confirmed this step yet; if you still need information from me, ask for it directly. Only finish this step and do not run later nodes; tell me the result and I will confirm completion in the workflow view.",
  },
  "workflowRun.stepCard.warning.empty_output": {
    "zh-CN": "本轮没有文本输出；该节点未声明输出契约，仍可完成。",
    "en-US":
      "This turn had no text output; the node declares no output contract, so it can still complete.",
  },
  "workflowRun.stepCard.warning.history_integrity_gap": {
    "zh-CN": "会话历史存在缺口，本轮结果可能不完整。",
    "en-US":
      "The session history has a gap; this turn's result may be incomplete.",
  },
  "workflowRun.stepCard.warning.history_unavailable": {
    "zh-CN": "无法读取会话历史，本轮结果未知。",
    "en-US":
      "The session history could not be read; this turn's result is unknown.",
  },
} as const;

const BLOCKER_COPY = {
  "workflowRun.stepCard.blocker.turn_stopped": {
    "zh-CN": "已停止：本轮生成被中断。",
    "en-US": "Stopped: generation was interrupted.",
  },
  "workflowRun.stepCard.blocker.turn_truncated": {
    "zh-CN": "被截断：本轮达到输出上限。",
    "en-US": "Truncated: the turn hit its output limit.",
  },
  "workflowRun.stepCard.blocker.turn_refused": {
    "zh-CN": "拒答：智能体没有完成本轮请求。",
    "en-US": "Refused: the agent declined the requested work.",
  },
  "workflowRun.stepCard.blocker.turn_unknown": {
    "zh-CN": "回合未知：没有找到本轮正常结束的记录。",
    "en-US": "Turn unknown: no normally ended turn was found.",
  },
  "workflowRun.stepCard.blocker.turn_unconfirmed": {
    "zh-CN": "首轮未确认：本轮已正常结束，但还没有你的确认。",
    "en-US":
      "First turn unconfirmed: the turn ended normally, but you have not confirmed the step yet.",
  },
  "workflowRun.stepCard.blocker.structured_invalid": {
    "zh-CN": "结构化输出不符合：本轮输出未通过声明的结构校验。",
    "en-US":
      "Structured output invalid: this turn did not match the declared schema.",
  },
  "workflowRun.stepCard.blocker.output_missing": {
    "zh-CN": "没有输出：本轮没有产生声明的输出。",
    "en-US": "No output: this turn produced none of the declared output.",
  },
} as const;

function lookup(
  locale: (typeof LOCALES)[number],
  key: string,
): string | undefined {
  return (workflowRunTranslations[locale] as Record<string, string>)[key];
}

function placeholders(text: string): string[] {
  return Array.from(text.matchAll(/\{\{(\w+)\}\}/gu), (match) => match[1]!)
    .sort()
    .filter((name, index, all) => all.indexOf(name) === index);
}

describe("interactive step card translations", () => {
  it.each(
    [
      ...Object.entries(STEP_CARD_COPY),
      ...Object.entries(BLOCKER_COPY),
    ].flatMap(([key, texts]) =>
      LOCALES.map((locale) => ({ key, locale, text: texts[locale] })),
    ),
  )("$locale $key has the exact copy", ({ key, locale, text }) => {
    expect(lookup(locale, key)).toBe(text);
  });

  it("covers every blocker, provisional state and paused badge in both languages", () => {
    for (const locale of LOCALES) {
      for (const key of [
        ...Object.keys(BLOCKER_COPY),
        "workflowRun.stepCard.provisional",
        "workflowRun.stepCard.pausedBadge",
        "workflowRun.stepCard.waitingBadge",
      ]) {
        expect(lookup(locale, key)).toBeTruthy();
      }
    }
  });

  it.each(
    [
      "workflowRun.stepCard.readyAction",
      "workflowRun.stepCard.continuePrompt",
    ].map((key) => ({ key })),
  )("$key uses the same placeholders in both languages", ({ key }) => {
    expect(placeholders(lookup("zh-CN", key)!)).toEqual(
      placeholders(lookup("en-US", key)!),
    );
  });
});
