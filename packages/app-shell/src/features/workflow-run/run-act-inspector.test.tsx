import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { createChatStore } from "@ora/chat";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import { createAgentMemory, agentHandlers } from "../../test/memory/agents";
import { createSkillMemory, skillHandlers } from "../../test/memory/skills";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { appI18n } from "../../i18n/i18n-instance";
import { RunActInspector } from "./run-act-inspector";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import type {
  GraphWorkflowNodeState,
  GraphWorkflowRunStatus,
  WorkflowNodeData,
} from "@ora/workflow-runtime";
import { AGENT_REF } from "../../test/agent-identity";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return {
    ...createPluginMemory(),
    ...createAgentMemory(),
    ...createSkillMemory(),
  };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...pluginHandlers(state),
    ...agentHandlers(state),
    ...skillHandlers(state),
  };
}

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

const START_DATA: WorkflowNodeData = {
  kind: "start",
  title: "开始",
  description: "接收输入",
};

/** Mounts the act inspector with catalog-backed Agent/Skill names. */
function renderInspector(
  nodeState: GraphWorkflowNodeState = { status: "succeeded" },
  options: {
    data?: WorkflowNodeData;
    handlers?: TestHandlers;
    runStatus?: GraphWorkflowRunStatus;
    runSnapshotId?: string;
    roundStates?: Record<string, GraphWorkflowNodeState[]>;
    selectedRound?: number | null;
  } = {},
) {
  useWorkspaceSelectionStore
    .getState()
    .selectWorkflowRun("run-1", "project-1");
  const state = createFixtureState();
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
  const clientHandlers: TestHandlers = {
    ...createFixtureHandlers(state),
    ...options.handlers,
  };
  const client = createTestClient(clientHandlers);
  const queryClient = createTestQueryClient();
  const Wrapper = createHookWrapper(
    client,
    queryClient,
    createChatStore(client.session),
  );

  return {
    user: userEvent.setup(),
    clientHandlers,
    ...render(
      <Wrapper>
        <RunActInspector
          nodeId="agent-1"
          data={options.data ?? AGENT_DATA}
          state={nodeState}
          roundStates={options.roundStates}
          selectedRound={options.selectedRound ?? null}
          onRoundChange={() => undefined}
          artifacts={[]}
          revealedArtifactId={null}
          onClose={() => undefined}
          runStatus={options.runStatus}
          runSnapshotId={options.runSnapshotId}
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

describe("RunActInspector failure detail", () => {
  it("renders the kind title, hint, and attempt line for a failed node", async () => {
    await appI18n.changeLanguage("zh-CN");
    renderInspector({
      status: "failed",
      errorMessage: "agent node review structured output failed: not json",
      errorDetail: {
        kind: "structured_output",
        message: "agent node review structured output failed: not json",
        sourceChain: ["not json"],
        attempt: 2,
        resumable: false,
        injectsPreviousFailure: false,
        recordedAt: 50,
      },
    });

    expect(await screen.findByText("结构化输出不合格")).toBeInTheDocument();
    expect(
      screen.getByText(
        "智能体的回复不符合输出结构；可直接续跑让它带着失败信息重试，或调整提示词/输出结构后发布新版本",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("第 2 次尝试")).toBeInTheDocument();
    expect(
      screen.getByText(
        "这类失败通常源于工作流本身，直接续跑很可能再次失败；建议修改工作流后重新运行。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("agent node review structured output failed: not json"),
    ).toBeInTheDocument();
  });

  it("omits the not-resumable hint when the failure is resumable", async () => {
    await appI18n.changeLanguage("zh-CN");
    renderInspector({
      status: "failed",
      errorMessage: '{"reason":"interrupted_by_restart"}',
      errorDetail: {
        kind: "interrupted_by_restart",
        message: '{"reason":"interrupted_by_restart"}',
        sourceChain: [],
        attempt: 1,
        resumable: true,
        injectsPreviousFailure: false,
        recordedAt: 80,
      },
    });

    expect(await screen.findByText("被应用重启打断")).toBeInTheDocument();
    expect(
      screen.getByText("应用重启时该节点仍在运行，可直接续跑"),
    ).toBeInTheDocument();
    expect(screen.getByText("第 1 次尝试")).toBeInTheDocument();
    expect(
      screen.queryByText(
        "这类失败通常源于工作流本身，直接续跑很可能再次失败；建议修改工作流后重新运行。",
      ),
    ).not.toBeInTheDocument();
  });

  it("renders the injected-resume hint for a structured_output failure", async () => {
    await appI18n.changeLanguage("zh-CN");
    renderInspector({
      status: "failed",
      errorMessage: "agent node review structured output failed: not json",
      errorDetail: {
        kind: "structured_output",
        message: "agent node review structured output failed: not json",
        sourceChain: ["not json"],
        attempt: 2,
        resumable: false,
        injectsPreviousFailure: true,
        recordedAt: 50,
      },
    });

    expect(
      await screen.findByText(
        "同版本续跑时，Ora 会把这次失败的类型、原因和上次输出告诉智能体让它重试；若仍失败，再修改工作流并发布新版本。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(
        "这类失败通常源于工作流本身，直接续跑很可能再次失败；建议修改工作流后重新运行。",
      ),
    ).not.toBeInTheDocument();
  });

  it("renders injected previous-failure context in a collapsed details element", async () => {
    await appI18n.changeLanguage("zh-CN");
    renderInspector({
      status: "running",
      injectedFailureContext: "## 上一次尝试（第 1 次）失败信息\n类型：结构化输出不合格",
    });
    expect(
      await screen.findByText("本次尝试注入的上次失败信息"),
    ).toBeInTheDocument();
    expect(document.querySelector("pre")?.textContent).toBe(
      "## 上一次尝试（第 1 次）失败信息\n类型：结构化输出不合格",
    );
  });

  it("omits injected previous-failure context when the node has none", async () => {
    await appI18n.changeLanguage("zh-CN");
    renderInspector({ status: "running" });
    expect(
      screen.queryByText("本次尝试注入的上次失败信息"),
    ).not.toBeInTheDocument();
  });
});

describe("RunActInspector run context hints", () => {
  it("renders the resume-from-top hint for a failed node on a failed run", async () => {
    await appI18n.changeLanguage("zh-CN");
    renderInspector(
      {
        status: "failed",
        errorMessage: "boom",
      },
      { runStatus: "failed" },
    );
    expect(
      await screen.findByText("可在顶部点「从失败处继续」重跑这个节点"),
    ).toBeInTheDocument();
  });

  it("omits the resume-from-top hint while the run is still running", async () => {
    await appI18n.changeLanguage("zh-CN");
    renderInspector(
      {
        status: "failed",
        errorMessage: "boom",
      },
      { runStatus: "running" },
    );
    expect(await screen.findByText("boom")).toBeInTheDocument();
    expect(
      screen.queryByText("可在顶部点「从失败处继续」重跑这个节点"),
    ).not.toBeInTheDocument();
  });

  it("renders the older-snapshot hint when node and run snapshot ids differ", async () => {
    await appI18n.changeLanguage("zh-CN");
    renderInspector(
      { status: "succeeded", snapshotId: "snap-old" },
      { runSnapshotId: "snap-new" },
    );
    expect(
      await screen.findByText(
        "此节点的结果来自本运行之前使用的版本（续跑时已切换版本）",
      ),
    ).toBeInTheDocument();
  });

  it("omits the older-snapshot hint when snapshot ids match or either is missing", async () => {
    await appI18n.changeLanguage("zh-CN");
    const matching = renderInspector(
      { status: "succeeded", snapshotId: "snap-1" },
      { runSnapshotId: "snap-1" },
    );
    expect(
      screen.queryByText(
        "此节点的结果来自本运行之前使用的版本（续跑时已切换版本）",
      ),
    ).not.toBeInTheDocument();
    matching.unmount();
    const missingRun = renderInspector({
      status: "succeeded",
      snapshotId: "snap-old",
    });
    expect(
      screen.queryByText(
        "此节点的结果来自本运行之前使用的版本（续跑时已切换版本）",
      ),
    ).not.toBeInTheDocument();
    missingRun.unmount();
    renderInspector({ status: "succeeded" }, { runSnapshotId: "snap-new" });
    expect(
      screen.queryByText(
        "此节点的结果来自本运行之前使用的版本（续跑时已切换版本）",
      ),
    ).not.toBeInTheDocument();
  });
});

describe("RunActInspector AI diagnosis", () => {
  it("renders the analyze button only for a failed agent node", async () => {
    await appI18n.changeLanguage("zh-CN");
    const succeeded = renderInspector({ status: "succeeded" });
    expect(
      screen.queryByRole("button", { name: "让 AI 分析" }),
    ).not.toBeInTheDocument();
    succeeded.unmount();
    const failedStart = renderInspector(
      { status: "failed", errorMessage: "boom" },
      { data: START_DATA },
    );
    expect(
      screen.queryByRole("button", { name: "让 AI 分析" }),
    ).not.toBeInTheDocument();
    failedStart.unmount();
    renderInspector({ status: "failed", errorMessage: "boom" });
    expect(
      await screen.findByRole("button", { name: "让 AI 分析" }),
    ).toBeInTheDocument();
  });

  it("calls diagnoseNodeFailure with the selected run and node once", async () => {
    await appI18n.changeLanguage("zh-CN");
    const diagnoseNodeFailure = vi.fn(
      async (request: { runId: string; nodeId: string }) => {
        expect(request).toEqual({ runId: "run-1", nodeId: "agent-1" });
        return {
          diagnosis: {
            text: "guess",
            agentCli: "open_code",
            model: "m",
            generatedAt: 1n,
          },
        };
      },
    );
    const { user } = renderInspector(
      { status: "failed", errorMessage: "boom" },
      { handlers: { diagnoseWorkflowNodeFailure: diagnoseNodeFailure } },
    );
    await user.click(await screen.findByRole("button", { name: "让 AI 分析" }));
    expect(diagnoseNodeFailure).toHaveBeenCalledTimes(1);
  });

  it("renders the guess title, text, and disclaimer when aiDiagnosis is present", async () => {
    await appI18n.changeLanguage("zh-CN");
    renderInspector({
      status: "failed",
      errorMessage: "boom",
      aiDiagnosis: {
        text: "模型推测根因是输出结构。",
        agentCli: "open_code",
        model: "deepseek/deepseek-v4-pro",
        generatedAt: 50,
      },
    });
    expect(
      await screen.findByText("AI 推测（deepseek/deepseek-v4-pro）"),
    ).toBeInTheDocument();
    expect(screen.getByText("模型推测根因是输出结构。")).toBeInTheDocument();
    expect(
      screen.getByText("这是模型的推测，不参与任何自动判断。"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "重新分析" }),
    ).toBeInTheDocument();
  });

  it("renders the selected round's error block and injected context", async () => {
    await appI18n.changeLanguage("zh-CN");
    const round0: GraphWorkflowNodeState = {
      status: "succeeded",
      iteration: 0,
      errorMessage: undefined,
    };
    const round1: GraphWorkflowNodeState = {
      status: "failed",
      iteration: 1,
      errorMessage: "round 2 exploded",
      errorDetail: {
        kind: "session",
        message: "round 2 exploded",
        sourceChain: [],
        attempt: 1,
        resumable: true,
        injectsPreviousFailure: true,
        recordedAt: 50,
      },
      injectedFailureContext: "## 上一次尝试（第 1 次）失败信息\n类型：会话失败",
    };
    renderInspector(round1, {
      runStatus: "failed",
      selectedRound: 1,
      roundStates: { "agent-1": [round0, round1] },
    });
    expect(await screen.findByText("round 2 exploded")).toBeInTheDocument();
    expect(screen.getByText("智能体会话失败")).toBeInTheDocument();
    expect(
      screen.getByText("本次尝试注入的上次失败信息"),
    ).toBeInTheDocument();
    expect(document.querySelector("pre")?.textContent).toBe(
      "## 上一次尝试（第 1 次）失败信息\n类型：会话失败",
    );
  });
});
