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
  if (FAILURE_PHRASES.some((phrase) => normalized.includes(phrase))) return "tool_failure";
  if (OPERATION_PHRASES.some((phrase) => normalized.includes(phrase))) return "tool_success";
  return "chat";
};

/** Returns whether the successful scenario should inspect the fixed test fixture. */
export function promptRequestsTestInspection(promptText: string): boolean {
  const normalized = promptText.toLocaleLowerCase();
  return normalized.includes("测试") || normalized.includes("test");
}
