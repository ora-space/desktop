export type MockCodingAgentId =
  | "codex"
  | "claude_code"
  | "open_code"
  | "code_agent";

export type MockModelCapability =
  | "recommended"
  | "coding"
  | "reasoning"
  | "balanced"
  | "fast"
  | "free"
  | "longContext"
  | "multimodal"
  | "agentic";

export interface MockModelOption {
  id: string;
  name: string;
  contextWindow: string;
  capabilities: readonly [
    MockModelCapability,
    ...MockModelCapability[],
  ];
}

export interface MockCodingAgent {
  id: MockCodingAgentId;
  name: string;
  models: readonly MockModelOption[];
}

export const DEFAULT_MOCK_MODEL_ID = "open-code/deepseek-v4-pro";

/**
 * Frontend-only catalog that mirrors recognizable coding agents and real model
 * families without coupling the visual prototype to runtime model discovery.
 */
export const MOCK_CODING_AGENTS: readonly MockCodingAgent[] = [
  {
    id: "open_code",
    name: "OpenCode",
    models: [
      {
        id: DEFAULT_MOCK_MODEL_ID,
        name: "DeepSeek V4 Pro",
        contextWindow: "256K",
        capabilities: ["recommended", "coding"],
      },
      {
        id: "open-code/gpt-5.6-sol",
        name: "GPT-5.6 Sol",
        contextWindow: "1.05M",
        capabilities: ["reasoning", "agentic"],
      },
      {
        id: "open-code/claude-opus-5",
        name: "Claude Opus 5",
        contextWindow: "1M",
        capabilities: ["agentic", "coding"],
      },
      {
        id: "open-code/gemini-3.6-flash",
        name: "Gemini 3.6 Flash",
        contextWindow: "1M",
        capabilities: ["fast", "multimodal"],
      },
      {
        id: "open-code/grok-4.5",
        name: "Grok 4.5",
        contextWindow: "256K",
        capabilities: ["reasoning", "agentic"],
      },
      {
        id: "open-code/kimi-k3",
        name: "Kimi K3",
        contextWindow: "256K",
        capabilities: ["agentic", "longContext"],
      },
      {
        id: "open-code/glm-5.2",
        name: "GLM-5.2",
        contextWindow: "200K",
        capabilities: ["coding", "reasoning"],
      },
      {
        id: "open-code/qwen3.7-max",
        name: "Qwen3.7 Max",
        contextWindow: "256K",
        capabilities: ["reasoning", "balanced"],
      },
      {
        id: "open-code/minimax-m3",
        name: "MiniMax M3",
        contextWindow: "256K",
        capabilities: ["agentic", "balanced"],
      },
      {
        id: "open-code/deepseek-v4-flash",
        name: "DeepSeek V4 Flash",
        contextWindow: "256K",
        capabilities: ["fast", "coding"],
      },
    ],
  },
  {
    id: "codex",
    name: "Codex",
    models: [
      {
        id: "codex/gpt-5.6-sol",
        name: "GPT-5.6 Sol",
        contextWindow: "1.05M",
        capabilities: ["recommended", "agentic"],
      },
      {
        id: "codex/gpt-5.6-terra",
        name: "GPT-5.6 Terra",
        contextWindow: "1.05M",
        capabilities: ["balanced", "coding"],
      },
      {
        id: "codex/gpt-5.6-luna",
        name: "GPT-5.6 Luna",
        contextWindow: "1.05M",
        capabilities: ["fast", "agentic"],
      },
      {
        id: "codex/gpt-5.5",
        name: "GPT-5.5",
        contextWindow: "1.05M",
        capabilities: ["reasoning", "coding"],
      },
      {
        id: "codex/gpt-5.5-pro",
        name: "GPT-5.5 Pro",
        contextWindow: "1.05M",
        capabilities: ["reasoning", "longContext"],
      },
      {
        id: "codex/gpt-5.4",
        name: "GPT-5.4",
        contextWindow: "1.05M",
        capabilities: ["coding", "balanced"],
      },
      {
        id: "codex/gpt-5.4-mini",
        name: "GPT-5.4 Mini",
        contextWindow: "400K",
        capabilities: ["fast", "agentic"],
      },
      {
        id: "codex/gpt-5.3-codex",
        name: "GPT-5.3 Codex",
        contextWindow: "400K",
        capabilities: ["coding", "agentic"],
      },
    ],
  },
  {
    id: "claude_code",
    name: "Claude Code",
    models: [
      {
        id: "claude-code/claude-fable-5",
        name: "Claude Fable 5",
        contextWindow: "1M",
        capabilities: ["recommended", "agentic"],
      },
      {
        id: "claude-code/claude-opus-5",
        name: "Claude Opus 5",
        contextWindow: "1M",
        capabilities: ["reasoning", "agentic"],
      },
      {
        id: "claude-code/claude-sonnet-5",
        name: "Claude Sonnet 5",
        contextWindow: "1M",
        capabilities: ["coding", "balanced"],
      },
      {
        id: "claude-code/claude-opus-4.8",
        name: "Claude Opus 4.8",
        contextWindow: "1M",
        capabilities: ["reasoning", "coding"],
      },
      {
        id: "claude-code/claude-opus-4.7",
        name: "Claude Opus 4.7",
        contextWindow: "1M",
        capabilities: ["agentic", "longContext"],
      },
      {
        id: "claude-code/claude-sonnet-4.6",
        name: "Claude Sonnet 4.6",
        contextWindow: "1M",
        capabilities: ["coding", "fast"],
      },
      {
        id: "claude-code/claude-haiku-4.5",
        name: "Claude Haiku 4.5",
        contextWindow: "200K",
        capabilities: ["fast", "balanced"],
      },
    ],
  },
  {
    id: "code_agent",
    name: "CodeAgent",
    models: [
      {
        id: "code-agent/minimax-m2.7",
        name: "MiniMax M2.7",
        contextWindow: "256K",
        capabilities: ["recommended", "coding"],
      },
      {
        id: "code-agent/glm-5.1",
        name: "GLM-5.1",
        contextWindow: "200K",
        capabilities: ["reasoning", "agentic"],
      },
    ],
  },
] as const;

/** Returns whether an id belongs to the frontend-only catalog. */
export function isMockModelId(modelId: string): boolean {
  return MOCK_CODING_AGENTS.some((agent) =>
    agent.models.some((model) => model.id === modelId),
  );
}

/** Resolves local display selection metadata and falls back to the recommended default. */
export function selectedMockModel(modelId: string) {
  for (const agent of MOCK_CODING_AGENTS) {
    const model = agent.models.find((candidate) => candidate.id === modelId);
    if (model !== undefined) return { agent, model };
  }
  const defaultSelection = MOCK_CODING_AGENTS.flatMap((agent) =>
    agent.models.map((model) => ({ agent, model })),
  ).find(({ model }) => model.id === DEFAULT_MOCK_MODEL_ID)!;
  return defaultSelection;
}
