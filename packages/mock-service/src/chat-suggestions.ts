/** Mock-only starter prompts that expose each deterministic ACP scenario. */
export const mockChatSuggestions = [
  {
    id: "mock-chat",
    text: {
      "zh-CN": "解释 greeting 的工作方式",
      "en-US": "Explain how greeting works",
    },
  },
  {
    id: "mock-tool-flow",
    text: {
      "zh-CN": "实现并验证 greeting",
      "en-US": "Implement and verify greeting",
    },
  },
  {
    id: "mock-tool-failure",
    text: {
      "zh-CN": "修改不存在的文件",
      "en-US": "Modify a nonexistent file",
    },
  },
] as const;
