import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MOCK_WORKFLOW, MockWorkflowRepository } from "../src";

describe("MockWorkflowRepository", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns isolated workflow copies", async () => {
    const repository = new MockWorkflowRepository();
    const load = repository.get(MOCK_WORKFLOW.id);
    await vi.runAllTimersAsync();
    const workflow = await load;

    workflow.nodes[0].title = "changed by consumer";
    const reload = repository.get(MOCK_WORKFLOW.id);
    await vi.runAllTimersAsync();

    await expect(reload).resolves.toEqual(MOCK_WORKFLOW);
  });

  it("saves edits in memory", async () => {
    const repository = new MockWorkflowRepository();
    const edited = {
      ...MOCK_WORKFLOW,
      name: "Edited workflow",
    };
    const save = repository.save(edited);
    await vi.runAllTimersAsync();
    await save;
    const reload = repository.get(edited.id);
    await vi.runAllTimersAsync();

    await expect(reload).resolves.toEqual(
      expect.objectContaining({
        ...edited,
        updatedAt: expect.any(String),
      }),
    );
  });

  it("creates a deterministic successful preview trace", async () => {
    const repository = new MockWorkflowRepository();
    const run = repository.run(MOCK_WORKFLOW.id, "Review the workspace");
    await vi.runAllTimersAsync();
    const result = await run;

    expect(result).toEqual({
      status: "success",
      durationMs: 1395,
      output:
        "已完成“代码审查工作流”的模拟运行。\n\n输入：Review the workspace\n\n发现 2 个建议项，未发现阻塞问题。",
      steps: [
        { nodeId: "start", durationMs: 140, summary: "开始 已完成" },
        { nodeId: "understand", durationMs: 177, summary: "理解改动 已完成" },
        { nodeId: "quality", durationMs: 214, summary: "质量门禁 已完成" },
        { nodeId: "tests", durationMs: 251, summary: "运行检查 已完成" },
        { nodeId: "review", durationMs: 288, summary: "审查 Agent 已完成" },
        { nodeId: "output", durationMs: 325, summary: "输出报告 已完成" },
      ],
    });
  });

  it("returns English fixture and run content for the English locale", async () => {
    const repository = new MockWorkflowRepository("en-US");
    const workflowLoad = repository.get(MOCK_WORKFLOW.id);
    await vi.runAllTimersAsync();
    const workflow = await workflowLoad;
    const run = repository.run(workflow.id, "");
    await vi.runAllTimersAsync();
    const result = await run;

    expect({
      workflowName: workflow.name,
      firstNodeTitle: workflow.nodes[0].title,
      firstStep: result.steps[0].summary,
      output: result.output,
    }).toEqual({
      workflowName: "Code review workflow",
      firstNodeTitle: "Start",
      firstStep: "Start completed",
      output:
        'Completed a simulated run of "Code review workflow".\n\nInput: Review uncommitted changes in the current workspace\n\nFound 2 suggestions and no blocking issues.',
    });
  });
});
