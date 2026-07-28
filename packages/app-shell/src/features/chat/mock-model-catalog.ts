export type MockModelProviderId =
  | "openai"
  | "anthropic"
  | "deepseek"
  | "opencode";

export type MockModelCapability =
  | "recommended"
  | "coding"
  | "reasoning"
  | "balanced"
  | "fast"
  | "free"
  | "longContext";

export interface MockModelOption {
  id: string;
  name: string;
  contextWindow: string;
  capabilities: readonly [
    MockModelCapability,
    ...MockModelCapability[],
  ];
}

export interface MockModelProvider {
  id: MockModelProviderId;
  name: string;
  models: readonly MockModelOption[];
}

export const DEFAULT_MOCK_MODEL_ID = "deepseek/deepseek-v4-pro";

/**
 * Frontend-only catalog used to make the model picker representative without
 * coupling visual prototypes to installed CLI discovery or runtime routing.
 */
export const MOCK_MODEL_PROVIDERS: readonly MockModelProvider[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    models: [
      {
        id: DEFAULT_MOCK_MODEL_ID,
        name: "DeepSeek V4 Pro",
        contextWindow: "256K",
        capabilities: ["recommended", "coding"],
      },
      {
        id: "deepseek/deepseek-v4-flash",
        name: "DeepSeek V4 Flash",
        contextWindow: "256K",
        capabilities: ["fast"],
      },
      {
        id: "deepseek/deepseek-reasoner",
        name: "DeepSeek Reasoner",
        contextWindow: "128K",
        capabilities: ["reasoning"],
      },
    ],
  },
  {
    id: "openai",
    name: "OpenAI",
    models: [
      {
        id: "openai/gpt-5.2-codex",
        name: "GPT-5.2 Codex",
        contextWindow: "400K",
        capabilities: ["recommended", "coding"],
      },
      {
        id: "openai/gpt-5.2",
        name: "GPT-5.2",
        contextWindow: "400K",
        capabilities: ["reasoning"],
      },
      {
        id: "openai/gpt-4.1",
        name: "GPT-4.1",
        contextWindow: "1M",
        capabilities: ["fast"],
      },
    ],
  },
  {
    id: "anthropic",
    name: "Claude",
    models: [
      {
        id: "anthropic/claude-opus-4.5",
        name: "Claude Opus 4.5",
        contextWindow: "200K",
        capabilities: ["reasoning"],
      },
      {
        id: "anthropic/claude-sonnet-4.5",
        name: "Claude Sonnet 4.5",
        contextWindow: "200K",
        capabilities: ["recommended", "balanced"],
      },
      {
        id: "anthropic/claude-haiku-4.5",
        name: "Claude Haiku 4.5",
        contextWindow: "200K",
        capabilities: ["fast"],
      },
    ],
  },
  {
    id: "opencode",
    name: "OpenCode",
    models: [
      {
        id: "opencode/big-pickle",
        name: "Big Pickle",
        contextWindow: "200K",
        capabilities: ["coding"],
      },
      {
        id: "opencode/glm-4.7-free",
        name: "GLM-4.7 Free",
        contextWindow: "128K",
        capabilities: ["free", "balanced"],
      },
      {
        id: "opencode/kimi-k2.5-free",
        name: "Kimi K2.5 Free",
        contextWindow: "256K",
        capabilities: ["free", "longContext"],
      },
    ],
  },
] as const;

/** Returns whether an id belongs to the frontend-only catalog. */
export function isMockModelId(modelId: string): boolean {
  return MOCK_MODEL_PROVIDERS.some((provider) =>
    provider.models.some((model) => model.id === modelId),
  );
}

/** Resolves local display selection metadata and falls back to the recommended default. */
export function selectedMockModel(modelId: string) {
  for (const provider of MOCK_MODEL_PROVIDERS) {
    const model = provider.models.find((candidate) => candidate.id === modelId);
    if (model !== undefined) return { provider, model };
  }
  const defaultSelection = MOCK_MODEL_PROVIDERS.flatMap((provider) =>
    provider.models.map((model) => ({ provider, model })),
  ).find(({ model }) => model.id === DEFAULT_MOCK_MODEL_ID)!;
  return {
    provider: defaultSelection.provider,
    model: defaultSelection.model,
  };
}
