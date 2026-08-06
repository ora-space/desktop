import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { Node } from "@xyflow/react";
import { createMockWorkflowCapabilities, type WorkflowNodeData } from "@ora/workflow-mock";
import { appI18n } from "../../i18n/i18n-instance";
import { AppI18nProvider } from "../../i18n/i18n";
import { WorkflowInspector } from "./workflow-inspector";

const LONG_MODEL_LABEL =
  "OpenCode · deepseek/deepseek-v4-pro-with-an-extremely-long-model-identifier";

/** Builds one Agent node whose long labels would overflow a narrow inspector without min-width constraints. */
function createAgentNode(): Node<WorkflowNodeData, "workflow"> {
  return {
    id: "agent-1",
    type: "workflow",
    position: { x: 0, y: 0 },
    data: {
      kind: "agent",
      title: "探索",
      description: "只读探索项目现状和影响范围",
      agentConfig: {
        schemaVersion: 3,
        executor: {
          agentCli: "open_code",
          modelId: "deepseek/deepseek-v4-pro-with-an-extremely-long-model-identifier",
        },
        roleId: "Researcher",
        skills: [{ skillId: "openspec-explore", enabled: true }],
        prompt: "阅读相关代码、文档和现有规范，输出现状、约束、风险与可选路径。",
      },
    },
  };
}

/** Mounts the inspector inside a fixed-width clip container that mirrors the settings rail. */
function renderNarrowInspector(): HTMLElement {
  const capabilities = createMockWorkflowCapabilities("zh-CN", [
    {
      agentCli: "open_code",
      modelId: "deepseek/deepseek-v4-pro-with-an-extremely-long-model-identifier",
      label: LONG_MODEL_LABEL,
    },
  ]);
  const container = document.createElement("div");
  container.dataset.testid = "narrow-inspector-host";
  container.style.width = "240px";
  container.style.overflow = "hidden";
  container.className = "flex min-h-0 min-w-0 flex-col";
  document.body.append(container);

  render(
    <AppI18nProvider>
      <WorkflowInspector
        node={createAgentNode()}
        capabilities={capabilities}
        onUpdate={() => undefined}
        onDelete={() => undefined}
        onCloseNode={() => undefined}
      />
    </AppI18nProvider>,
    { container },
  );
  return container;
}

describe("WorkflowInspector layout", () => {
  it("keeps picker chevrons and skill controls inside a narrow clipped rail", async () => {
    await appI18n.changeLanguage("zh-CN");
    const host = renderNarrowInspector();
    const inspector = host.querySelector("[data-workflow-inspector]");
    expect(inspector).not.toBeNull();
    expect(inspector).toHaveClass("min-w-0", "w-full", "overflow-hidden");

    const modelTrigger = screen.getByLabelText("Agent 模型");
    expect(modelTrigger).toHaveClass("min-w-0", "shrink", "overflow-hidden", "w-full");
    expect(within(modelTrigger).getByTestId("workflow-agent-model-chevron")).toBeInTheDocument();

    const roleTrigger = screen.getByLabelText("角色");
    expect(roleTrigger).toHaveClass("min-w-0", "shrink", "overflow-hidden", "w-full");
    expect(within(roleTrigger).getByTestId("workflow-agent-role-chevron")).toBeInTheDocument();

    const addSkill = screen.getByRole("button", { name: "添加 Skill" });
    expect(addSkill).toBeInTheDocument();
    expect(screen.getByText(/1\/1/)).toBeInTheDocument();
    expect(screen.getByRole("switch", {
      name: "启用或禁用 openspec-explore",
    })).toBeInTheDocument();
    expect(screen.getByRole("button", {
      name: "移除 openspec-explore",
    })).toBeInTheDocument();

    host.remove();
  });
});
