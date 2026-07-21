/** Identifies one deterministic behavior exposed by the mock ACP agent. */
export type MockAcpScenario = "chat" | "tool_success" | "tool_failure";

/** Selects a mock behavior from the normalized text of one user prompt. */
export type MockAcpScenarioResolver = (promptText: string) => MockAcpScenario;

const FAILURE_PHRASES = [
  "不存在的文件",
  "missing file",
  "nonexistent file",
  "non-existent file",
];

const READ_ONLY_PREFIXES = [
  "总结",
  "解释",
  "概述",
  "说明",
  "summarize ",
  "explain ",
  "describe ",
];

const OPERATION_PHRASES = [
  "修改",
  "实现",
  "修复",
  "重构",
  "创建",
  "modify",
  "implement",
  "fix",
  "refactor",
  "create",
];

/** Uses a deliberately small bilingual phrase list to keep mock routing predictable. */
export const defaultMockAcpScenarioResolver: MockAcpScenarioResolver = (promptText) => {
  const normalized = promptText.toLocaleLowerCase();
  const intent = normalized.trim().replace(/^(?:请(?:帮我)?|please)\s*/, "");
  if (READ_ONLY_PREFIXES.some((prefix) => intent.startsWith(prefix))) return "chat";
  if (FAILURE_PHRASES.some((phrase) => normalized.includes(phrase))) return "tool_failure";
  if (OPERATION_PHRASES.some((phrase) => normalized.includes(phrase))) return "tool_success";
  return "chat";
};
