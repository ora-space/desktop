import { describe, expect, it } from "vitest";
import {
  formatAgentExecutorLabel,
  resolveAgentRetryDisplay,
  resolveTheaterActDetail,
  resolveTheaterActInstruction,
} from "./agent-config-display";
import type {
  WorkflowAgentConfig,
  WorkflowNodeData,
} from "@ora/workflow-runtime";
import type { AgentEntry } from "../../state/hooks/use-agent-catalog";
import { AGENT_REF } from "../../test/agent-identity";

/** The installed agent packages these summaries are rendered against. */
const AGENTS: AgentEntry[] = [
  { agentRef: AGENT_REF.opencode, label: "OpenCode", logo: null },
];

describe("agent-config-display", () => {
  it("names the agent its installed package declares in the mono summary line", () => {
    expect(
      formatAgentExecutorLabel(
        {
          agentCli: AGENT_REF.opencode,
          modelId: "deepseek/deepseek-v4-pro",
        },
        AGENTS,
      ),
    ).toBe("OpenCode · deepseek/deepseek-v4-pro");
  });

  it("falls back to agent executor when flat detail fields are empty", () => {
    const data: WorkflowNodeData = {
      kind: "agent",
      title: "探索",
      description: "只读探索",
      agentConfig: {
        schemaVersion: 3,
        executor: {
          agentCli: AGENT_REF.opencode,
          modelId: "deepseek/deepseek-v4-flash",
        },
        roleId: "researcher",
        skills: [],
        mcps: [],
        prompt: "梳理现状与风险。",
      },
    };
    expect(resolveTheaterActDetail(data, AGENTS)).toBe(
      "OpenCode · deepseek/deepseek-v4-flash",
    );
    expect(resolveTheaterActInstruction(data)).toBe("梳理现状与风险。");
  });

  it("keeps the theater detail line quiet when the agent config carries no executor", () => {
    const data: WorkflowNodeData = {
      kind: "agent",
      title: "Legacy",
      description: "Minimal snapshot node",
      agentConfig: {
        schemaVersion: 3,
        prompt: "剩余提示词。",
      } as unknown as WorkflowAgentConfig,
    };
    expect(resolveTheaterActDetail(data, AGENTS)).toBe("—");
    expect(resolveTheaterActInstruction(data)).toBe("剩余提示词。");
  });

  it("falls back to the raw identity when no installed package names the agent", () => {
    expect(
      formatAgentExecutorLabel(
        { agentCli: "acme.my-agent", modelId: "acme/one" },
        AGENTS,
      ),
    ).toBe("acme.my-agent · acme/one");
  });

  it("degrades to em dashes when a snapshot executor or its identities are missing", () => {
    // Imported snapshots can predate the executor contract, so every missing
    // component must render a quiet placeholder instead of "undefined".
    expect(formatAgentExecutorLabel(undefined, AGENTS)).toBe("—");
    expect(
      formatAgentExecutorLabel(
        undefined as unknown as WorkflowAgentConfig["executor"],
        AGENTS,
      ),
    ).toBe("—");
    expect(
      formatAgentExecutorLabel(
        {
          agentCli: undefined,
          modelId: undefined,
        } as unknown as WorkflowAgentConfig["executor"],
        AGENTS,
      ),
    ).toBe("— · —");
  });

  it("prefers flat tool/condition and instruction over agentConfig", () => {
    const data: WorkflowNodeData = {
      kind: "agent",
      title: "Review",
      description: "Review branch",
      instruction: "Find regressions.",
      tool: "Terminal",
      condition: "contains source changes",
      agentConfig: {
        schemaVersion: 3,
        executor: { agentCli: AGENT_REF.opencode, modelId: "ignored" },
        roleId: "reviewer",
        skills: [],
        mcps: [],
        prompt: "Unused prompt",
      },
    };
    expect(resolveTheaterActDetail(data, AGENTS)).toBe("Terminal");
    expect(resolveTheaterActInstruction(data)).toBe("Find regressions.");
  });
});

describe("resolveAgentRetryDisplay", () => {
  const base: WorkflowAgentConfig = {
    schemaVersion: 3,
    executor: { agentCli: AGENT_REF.opencode, modelId: "m1" },
    roleId: "",
    skills: [],
    mcps: [],
    prompt: "",
  };

  it("reports the default policy when the snapshot has no retry field", () => {
    expect(resolveAgentRetryDisplay(base)).toEqual({
      kind: "enabled",
      source: "default",
      maxRetries: 2,
      initialDelaySeconds: 10,
    });
    expect(
      resolveAgentRetryDisplay({
        ...base,
        retry: null as unknown as WorkflowAgentConfig["retry"],
      }),
    ).toEqual({
      kind: "enabled",
      source: "default",
      maxRetries: 2,
      initialDelaySeconds: 10,
    });
  });

  it("reports the author's values when retry is configured", () => {
    expect(
      resolveAgentRetryDisplay({
        ...base,
        retry: { enabled: true, maxRetries: 5, initialDelaySeconds: 0 },
      }),
    ).toEqual({
      kind: "enabled",
      source: "configured",
      maxRetries: 5,
      initialDelaySeconds: 0,
    });
  });

  it("marks a configured policy that equals the default as configured", () => {
    expect(
      resolveAgentRetryDisplay({
        ...base,
        retry: { enabled: true, maxRetries: 2, initialDelaySeconds: 10 },
      }),
    ).toMatchObject({ kind: "enabled", source: "configured" });
  });

  it("reports a turned-off policy", () => {
    expect(
      resolveAgentRetryDisplay({
        ...base,
        retry: { enabled: false, maxRetries: 3, initialDelaySeconds: 30 },
      }),
    ).toEqual({ kind: "disabled" });
  });

  it("reports an enabled policy with zero retries as never rerunning", () => {
    expect(
      resolveAgentRetryDisplay({
        ...base,
        retry: { enabled: true, maxRetries: 0, initialDelaySeconds: 10 },
      }),
    ).toEqual({ kind: "noRetries" });
  });

  it("reports interactive nodes as never retried, whatever the stored policy", () => {
    expect(resolveAgentRetryDisplay({ ...base, interactive: true })).toEqual({
      kind: "interactive",
    });
    expect(
      resolveAgentRetryDisplay({
        ...base,
        interactive: true,
        retry: { enabled: true, maxRetries: 5, initialDelaySeconds: 60 },
      }),
    ).toEqual({ kind: "interactive" });
    expect(
      resolveAgentRetryDisplay({ ...base, interactive: false }),
    ).toMatchObject({ kind: "enabled", source: "default" });
  });
});
