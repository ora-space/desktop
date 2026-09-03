import { DEMO_AGENT_REF } from "../src/agent-identity";
import { describe, expect, it } from "vitest";
import type { Node } from "@xyflow/react";
import {
  normalizeWorkflowAgentConfig,
  normalizeWorkflowNodeAgentConfigs,
} from "../src/normalize-agent-config";
import type { WorkflowAgentConfig, WorkflowNodeData } from "../src/node-data";

describe("normalizeWorkflowAgentConfig", () => {
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
      outputPolicy: "none",
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
});
