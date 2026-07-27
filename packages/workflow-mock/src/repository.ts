import { createMockWorkflow } from "./fixtures";
import type {
  WorkflowDefinition,
  WorkflowRepository,
  WorkflowRunResult,
  WorkflowLocale,
} from "./types";

const MOCK_LATENCY_MS = 180;

/** Creates an isolated copy so UI edits cannot mutate the fixture or another consumer's state. */
function cloneWorkflow(workflow: WorkflowDefinition): WorkflowDefinition {
  return structuredClone(workflow);
}

/** Simulates the future workflow API while keeping all prototype state inside this package. */
export class MockWorkflowRepository implements WorkflowRepository {
  private workflows: Map<string, WorkflowDefinition>;

  public constructor(private readonly locale: WorkflowLocale = "zh-CN") {
    const workflow = createMockWorkflow(locale);
    this.workflows = new Map([[workflow.id, workflow]]);
  }

  /** Returns every mock workflow after a short delay so loading states stay testable. */
  async list(): Promise<WorkflowDefinition[]> {
    await waitForMockLatency();
    return [...this.workflows.values()].map(cloneWorkflow);
  }

  /** Loads one editable workflow or reports the same missing-resource shape a backend would. */
  async get(id: string): Promise<WorkflowDefinition> {
    await waitForMockLatency();
    const workflow = this.workflows.get(id);
    if (workflow === undefined) {
      throw new Error(`Workflow "${id}" was not found`);
    }
    return cloneWorkflow(workflow);
  }

  /** Persists changes in memory for the lifetime of the current application process. */
  async save(workflow: WorkflowDefinition): Promise<WorkflowDefinition> {
    await waitForMockLatency();
    const saved = {
      ...cloneWorkflow(workflow),
      updatedAt: new Date().toISOString(),
    };
    this.workflows.set(saved.id, saved);
    return cloneWorkflow(saved);
  }

  /** Produces deterministic preview output without executing tools or contacting a model. */
  async run(id: string, input: string): Promise<WorkflowRunResult> {
    const workflow = await this.get(id);
    await waitForMockLatency();
    const steps = workflow.nodes.map((node, index) => ({
      nodeId: node.id,
      durationMs: 140 + index * 37,
      summary: this.locale === "zh-CN" ? `${node.title} 已完成` : `${node.title} completed`,
    }));
    const durationMs = steps.reduce((total, step) => total + step.durationMs, 0);
    return {
      status: "success",
      durationMs,
      output: this.locale === "zh-CN"
        ? `已完成“${workflow.name}”的模拟运行。\n\n输入：${input || "检查当前工作区的未提交改动"}\n\n发现 2 个建议项，未发现阻塞问题。`
        : `Completed a simulated run of "${workflow.name}".\n\nInput: ${input || "Review uncommitted changes in the current workspace"}\n\nFound 2 suggestions and no blocking issues.`,
      steps,
    };
  }
}

/** Keeps mock timing in one place so it can be replaced or disabled in tests later. */
function waitForMockLatency(): Promise<void> {
  return new Promise((resolve) => globalThis.setTimeout(resolve, MOCK_LATENCY_MS));
}
