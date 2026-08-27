import { describe, expect, it } from "vitest";
import {
  formatAgentExecutorLabel,
  resolveTheaterActDetail,
  resolveTheaterActInstruction,
} from "./agent-config-display";
import type { WorkflowNodeData } from "@ora/workflow-runtime";
import type { AgentEntry } from "../chat/agent-catalog";

/** The installed agent packages these summaries are rendered against. */
const AGENTS: AgentEntry[] = [
  { agentRef: "ora-space.opencode", label: "OpenCode", logo: null },
];

describe("agent-config-display", () => {
  it("names the agent its installed package declares in the mono summary line", () => {
    expect(
      formatAgentExecutorLabel(
        {
          agentCli: "ora-space.opencode",
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
          agentCli: "ora-space.opencode",
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

  it("falls back to the raw identity when no installed package names the agent", () => {
    expect(
      formatAgentExecutorLabel(
        { agentCli: "acme.my-agent", modelId: "acme/one" },
        AGENTS,
      ),
    ).toBe("acme.my-agent · acme/one");
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
        executor: { agentCli: "ora-space.opencode", modelId: "ignored" },
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
