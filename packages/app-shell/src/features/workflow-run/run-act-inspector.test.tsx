import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { createChatStore } from "@ora/chat";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { appI18n } from "../../i18n/i18n-instance";
import { RunActInspector } from "./run-act-inspector";
import type { WorkflowNodeData } from "@ora/workflow-runtime";
import { AGENT_REF } from "../../test/agent-identity";

const AGENT_DATA: WorkflowNodeData = {
  kind: "agent",
  title: "探索",
  description: "只读探索项目现状",
  agentConfig: {
    schemaVersion: 3,
    executor: {
      agentCli: AGENT_REF.opencode,
      modelId: "deepseek/deepseek-v4-pro",
    },
    roleId: "研究员",
    skills: [
      { skillId: "openspec-explore", enabled: true },
      { skillId: "hidden-skill", enabled: false },
    ],
    mcps: [],
    prompt: "阅读相关代码并输出风险。",
  },
};

/** Mounts the act inspector with catalog-backed Agent/Skill names. */
function renderInspector() {
  const state = createMockClientState();
  state.agents = [
    {
      id: "ag-researcher",
      namespace: "local",
      name: "研究员",
      description: "只读探索项目现状和影响范围",
    },
  ];
  state.skills = [
    {
      id: "sk-explore",
      namespace: "local",
      name: "openspec-explore",
      description: "探索仓库结构与约束",
      source: { kind: "local" } as const,
      availability: "available",
    },
    {
      id: "sk-disabled",
      namespace: "local",
      name: "hidden-skill",
      description: "Should not appear",
      source: { kind: "local" } as const,
      availability: "available",
    },
  ];
  const client = createMockClient(state);
  const queryClient = createTestQueryClient();
  const Wrapper = createHookWrapper(
    client,
    queryClient,
    createChatStore(client.session),
  );

  return {
    user: userEvent.setup(),
    ...render(
      <Wrapper>
        <RunActInspector
          nodeId="agent-1"
          data={AGENT_DATA}
          state={{ status: "succeeded" }}
          artifacts={[]}
          revealedArtifactId={null}
          onClose={() => undefined}
        />
      </Wrapper>,
    ),
  };
}

describe("RunActInspector agent config", () => {
  it("shows read-only agent fields and skill briefs for enabled skills only", async () => {
    await appI18n.changeLanguage("zh-CN");
    const { user } = renderInspector();

    await waitFor(() => {
      expect(
        screen.getByText("OpenCode · deepseek/deepseek-v4-pro"),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "查看角色「研究员」简介" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", {
          name: "查看 Skill「openspec-explore」简介",
        }),
      ).toBeInTheDocument();
    });
    expect(screen.getByText("阅读相关代码并输出风险。")).toBeInTheDocument();
    expect(screen.queryByText("hidden-skill")).not.toBeInTheDocument();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "查看角色「研究员」简介" }),
    );
    expect(
      await screen.findByText("只读探索项目现状和影响范围"),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", {
        name: "查看 Skill「openspec-explore」简介",
      }),
    );
    expect(await screen.findByText("探索仓库结构与约束")).toBeInTheDocument();
  });
});
