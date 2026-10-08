import { DEMO_AGENT_REF } from "../src/agent-identity";
import { describe, expect, it } from "vitest";
import type { Node } from "@xyflow/react";
import {
  normalizeWorkflowAgentConfig,
  normalizeWorkflowNodeAgentConfigs,
} from "../src/normalize-agent-config";
import type { WorkflowAgentConfig, WorkflowNodeData } from "../src/node-data";

describe("normalizeWorkflowAgentConfig", () => {
  it("keeps valid bindings and text while making malformed imported fields safe to display", () => {
    const imported = JSON.parse(
      JSON.stringify({
        schemaVersion: 3,
        executor: { modelId: {} },
        roleId: {},
        prompt: [],
        interactive: "true",
        skills: [
          null,
          { skillId: "unsafe", enabled: "true" },
          { skillId: "s1", enabled: false },
        ],
        mcps: {},
        retry: { enabled: false, maxRetries: 1, initialDelaySeconds: 0 },
      }),
    ) as WorkflowAgentConfig;
    const before = JSON.stringify(imported);
    expect(normalizeWorkflowAgentConfig(imported)).toEqual({
      schemaVersion: 3,
      executor: { agentCli: "", modelId: "" },
      roleId: "",
      prompt: "",
      interactive: false,
      skills: [{ skillId: "s1", enabled: false }],
      mcps: [],
      retry: { enabled: false, maxRetries: 1, initialDelaySeconds: 0 },
    });
    expect(JSON.stringify(imported)).toBe(before);
  });

  it("uses empty identities when the executor is absent, without choosing an installed agent", () => {
    const imported = JSON.parse(
      '{"schemaVersion":3,"prompt":"Review"}',
    ) as WorkflowAgentConfig;
    expect(normalizeWorkflowAgentConfig(imported)).toEqual({
      schemaVersion: 3,
      executor: { agentCli: "", modelId: "" },
      roleId: "",
      prompt: "Review",
      interactive: false,
      skills: [],
      mcps: [],
    });
  });

  it("fills omitted mcps and skills with empty arrays", () => {
    const legacy = {
      schemaVersion: 3 as const,
      executor: { agentCli: DEMO_AGENT_REF.opencode, modelId: "m1" },
      roleId: "Researcher",
      prompt: "hello",
    } as unknown as WorkflowAgentConfig;

    expect(normalizeWorkflowAgentConfig(legacy)).toEqual({
      schemaVersion: 3,
      executor: { agentCli: DEMO_AGENT_REF.opencode, modelId: "m1" },
      roleId: "Researcher",
      skills: [],
      mcps: [],
      prompt: "hello",
      interactive: false,
    });
  });

  it("normalizes agent nodes inside a graph envelope", () => {
    const nodes = [
      {
        id: "agent-1",
        type: "workflow" as const,
        position: { x: 0, y: 0 },
        data: {
          kind: "agent" as const,
          title: "探索",
          description: "desc",
          agentConfig: {
            schemaVersion: 3 as const,
            executor: { agentCli: DEMO_AGENT_REF.opencode, modelId: "m1" },
            roleId: "Researcher",
            skills: [{ skillId: "s1", enabled: true }],
            prompt: "p",
          } as unknown as WorkflowAgentConfig,
        },
      },
    ] satisfies Node<WorkflowNodeData, "workflow">[];

    const [normalized] = normalizeWorkflowNodeAgentConfigs(nodes);
    expect(normalized?.data.agentConfig?.mcps).toEqual([]);
    expect(normalized?.data.agentConfig?.skills).toEqual([
      { skillId: "s1", enabled: true },
    ]);
  });

  it("leaves an absent retry absent so saved graphs are not rewritten", () => {
    const config: WorkflowAgentConfig = {
      schemaVersion: 3,
      executor: { agentCli: DEMO_AGENT_REF.opencode, modelId: "m1" },
      roleId: "",
      skills: [],
      mcps: [],
      prompt: "p",
      interactive: false,
    };

    const normalized = normalizeWorkflowAgentConfig(config);

    expect(normalized).not.toHaveProperty("retry");
    expect(normalized).not.toHaveProperty("promptInactivity");
    // Retry adds nothing: an already-normalized graph serializes byte-for-byte the same after
    // another pass. (Legacy graphs still gain `interactive: false`; that predates retry.)
    expect(JSON.stringify(normalized)).toBe(JSON.stringify(config));
  });

  it("keeps present retry values untouched, including a turned-off policy", () => {
    const retry = { enabled: false, maxRetries: 4, initialDelaySeconds: 45 };
    const normalized = normalizeWorkflowAgentConfig({
      schemaVersion: 3,
      executor: { agentCli: DEMO_AGENT_REF.opencode, modelId: "m1" },
      roleId: "",
      skills: [],
      mcps: [],
      prompt: "p",
      retry,
    });

    expect(normalized.retry).toEqual({
      enabled: false,
      maxRetries: 4,
      initialDelaySeconds: 45,
    });
  });

  it.each([null, "timeout", "wait"] as const)(
    "keeps the saved inactivity policy %s without rewriting it",
    (promptInactivity) => {
      const config: WorkflowAgentConfig = {
        schemaVersion: 3,
        executor: { agentCli: DEMO_AGENT_REF.opencode, modelId: "m1" },
        roleId: "",
        skills: [],
        mcps: [],
        prompt: "p",
        interactive: false,
        promptInactivity,
      };

      expect(normalizeWorkflowAgentConfig(config)).toEqual(config);
    },
  );

  it("does not add or drop retry on agent nodes inside a graph envelope", () => {
    const agentConfig = (
      retry?: WorkflowAgentConfig["retry"],
    ): WorkflowAgentConfig => ({
      schemaVersion: 3,
      executor: { agentCli: DEMO_AGENT_REF.opencode, modelId: "m1" },
      roleId: "",
      skills: [],
      mcps: [],
      prompt: "p",
      ...(retry === undefined ? {} : { retry }),
    });
    const node = (
      id: string,
      config: WorkflowAgentConfig,
    ): Node<WorkflowNodeData, "workflow"> => ({
      id,
      type: "workflow",
      position: { x: 0, y: 0 },
      data: {
        kind: "agent",
        title: id,
        description: "",
        agentConfig: config,
      },
    });

    const [withoutRetry, withRetry] = normalizeWorkflowNodeAgentConfigs([
      node("plain", agentConfig()),
      node(
        "tuned",
        agentConfig({ enabled: true, maxRetries: 5, initialDelaySeconds: 0 }),
      ),
    ]);

    expect(withoutRetry?.data.agentConfig).not.toHaveProperty("retry");
    expect(withRetry?.data.agentConfig?.retry).toEqual({
      enabled: true,
      maxRetries: 5,
      initialDelaySeconds: 0,
    });
  });
});
