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

  it("creates, lists, and deletes workflows", async () => {
    const repository = new MockWorkflowRepository();
    const initialList = repository.list();
    await vi.runAllTimersAsync();
    const createdRequest = repository.create("新工作流");
    await vi.runAllTimersAsync();
    const created = await createdRequest;
    const populatedList = repository.list();
    await vi.runAllTimersAsync();

    expect({
      initial: await initialList,
      created,
      populated: await populatedList,
    }).toEqual({
      initial: expect.arrayContaining([
        expect.objectContaining({ id: "code-review" }),
        expect.objectContaining({ id: "release-readiness" }),
        expect.objectContaining({ id: "issue-triage" }),
      ]),
      created: expect.objectContaining({
        id: "workflow-1",
        name: "新工作流",
        nodes: [expect.objectContaining({ id: "start", kind: "start" })],
      }),
      populated: expect.arrayContaining([
        expect.objectContaining({ id: "workflow-1", name: "新工作流" }),
      ]),
    });

    const deletion = repository.delete(created.id);
    await vi.runAllTimersAsync();
    await deletion;
    const finalList = repository.list();
    await vi.runAllTimersAsync();

    expect(await finalList).not.toContainEqual(expect.objectContaining({ id: created.id }));
  });

  it("imports valid definitions with a unique id and rejects malformed JSON values", async () => {
    const repository = new MockWorkflowRepository();
    const importRequest = repository.importDefinition(MOCK_WORKFLOW);
    await vi.runAllTimersAsync();
    const imported = await importRequest;

    expect(imported).toEqual(expect.objectContaining({
      id: "code-review-imported-1",
      name: MOCK_WORKFLOW.name,
      updatedAt: expect.any(String),
    }));

    const invalidImport = expect(
      repository.importDefinition({ id: "broken" }),
    ).rejects.toThrow("Invalid workflow definition");
    await vi.runAllTimersAsync();

    await invalidImport;
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
