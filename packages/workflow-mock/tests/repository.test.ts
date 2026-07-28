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
        expect.objectContaining({ id: "openspec-change" }),
        expect.objectContaining({ id: "code-review" }),
        expect.objectContaining({ id: "ci-recovery" }),
        expect.objectContaining({ id: "release-readiness" }),
        expect.objectContaining({ id: "issue-triage" }),
        expect.objectContaining({ id: "dependency-update" }),
      ]),
      created: expect.objectContaining({
        id: "workflow-1",
        name: "新工作流",
        nodes: [expect.objectContaining({ id: "start", kind: "trigger" })],
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

  it("provides distinct realistic graphs for every preset workflow", async () => {
    const repository = new MockWorkflowRepository();
    const list = repository.list();
    await vi.runAllTimersAsync();
    const workflows = await list;

    expect(workflows.map((workflow) => ({
      id: workflow.id,
      nodeIds: workflow.nodes.map((node) => node.id),
    }))).toEqual([
      {
        id: "openspec-change",
        nodeIds: [
          "openspec-request",
          "openspec-explore",
          "openspec-propose",
          "openspec-apply",
          "openspec-sync",
          "openspec-archive",
          "openspec-summary",
        ],
      },
      {
        id: "code-review",
        nodeIds: ["start", "understand", "quality", "tests", "review", "output"],
      },
      {
        id: "ci-recovery",
        nodeIds: ["ci-input", "ci-logs", "ci-classify", "ci-retry", "ci-fix", "ci-result"],
      },
      {
        id: "release-readiness",
        nodeIds: [
          "release-input",
          "release-notes",
          "release-validate",
          "release-gate",
          "release-plan",
          "release-hold",
          "release-decision",
        ],
      },
      {
        id: "issue-triage",
        nodeIds: [
          "triage-intake",
          "triage-normalize",
          "triage-duplicates",
          "triage-severity",
          "triage-incident",
          "triage-backlog",
          "triage-output",
        ],
      },
      {
        id: "dependency-update",
        nodeIds: [
          "deps-scan",
          "deps-plan",
          "deps-update",
          "deps-verify",
          "deps-output",
        ],
      },
    ]);
    expect(workflows.every((workflow) =>
      workflow.nodes.every((node) => node.config.instruction.length >= 20)
    )).toBe(true);
    expect(workflows.every((workflow) =>
      workflow.nodes
        .filter((node) => node.kind === "tool" || node.kind === "code")
        .every((node) => (node.config.command?.length ?? 0) > 0)
    )).toBe(true);
  });

  it("imports valid definitions with a unique id and rejects malformed JSON values", async () => {
    const repository = new MockWorkflowRepository();
    const importRequest = repository.importDefinition(MOCK_WORKFLOW);
    await vi.runAllTimersAsync();
    const imported = await importRequest;

    expect(imported).toEqual(expect.objectContaining({
      id: "openspec-change-imported-1",
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
      durationMs: 1757,
      output:
        "已完成“OpenSpec 模式”的模拟运行。\n\n输入：Review the workspace\n\n变更：improve-workflow-mocks\n\nproposal、delta specs、design 与 tasks 已完成；实现通过格式化和测试，主规格已同步，变更已归档。",
      steps: [
        { nodeId: "openspec-request", durationMs: 140, summary: "开始变更 已完成" },
        { nodeId: "openspec-explore", durationMs: 177, summary: "探索需求 已完成" },
        { nodeId: "openspec-propose", durationMs: 214, summary: "创建提案 已完成" },
        { nodeId: "openspec-apply", durationMs: 251, summary: "实施变更 已完成" },
        { nodeId: "openspec-sync", durationMs: 288, summary: "同步主规格 已完成" },
        { nodeId: "openspec-archive", durationMs: 325, summary: "归档变更 已完成" },
        { nodeId: "openspec-summary", durationMs: 362, summary: "输出变更摘要 已完成" },
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
      workflowName: "OpenSpec mode",
      firstNodeTitle: "Start change",
      firstStep: "Start change completed",
      output:
        'Completed a simulated run of "OpenSpec mode".\n\nInput: Create and implement an OpenSpec change for the current request\n\nChange: improve-workflow-mocks\n\nProposal, delta specs, design, and tasks are complete. Formatting and tests passed, main specs were synced, and the change was archived.',
    });
  });

  it("uses a realistic successful path and output for the OpenSpec preview", async () => {
    const repository = new MockWorkflowRepository();
    const run = repository.run("openspec-change", "为设置页补齐工作流 mock");
    await vi.runAllTimersAsync();
    const result = await run;

    expect({
      status: result.status,
      nodeIds: result.steps.map((step) => step.nodeId),
      output: result.output,
    }).toEqual({
      status: "success",
      nodeIds: [
        "openspec-request",
        "openspec-explore",
        "openspec-propose",
        "openspec-apply",
        "openspec-sync",
        "openspec-archive",
        "openspec-summary",
      ],
      output:
        "已完成“OpenSpec 模式”的模拟运行。\n\n输入：为设置页补齐工作流 mock\n\n变更：improve-workflow-mocks\n\nproposal、delta specs、design 与 tasks 已完成；实现通过格式化和测试，主规格已同步，变更已归档。",
    });
  });
});
