import { createMockWorkflows } from "./fixtures";
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
  private nextWorkflowId = 1;

  public constructor(private readonly locale: WorkflowLocale = "zh-CN") {
    const workflows = createMockWorkflows(locale);
    this.workflows = new Map(workflows.map((workflow) => [workflow.id, workflow]));
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

  /** Creates a blank workflow with a usable Start node so the graph is never structurally empty. */
  async create(name: string): Promise<WorkflowDefinition> {
    await waitForMockLatency();
    const id = `workflow-${this.nextWorkflowId++}`;
    const workflow: WorkflowDefinition = {
      id,
      name,
      description: this.locale === "zh-CN" ? "尚未添加描述" : "No description yet",
      updatedAt: new Date().toISOString(),
      nodes: [
        {
          id: "start",
          kind: "start",
          title: this.locale === "zh-CN" ? "开始" : "Start",
          description: this.locale === "zh-CN" ? "接收工作流输入" : "Receive workflow input",
          position: { x: 120, y: 260 },
          config: {
            instruction: this.locale === "zh-CN"
              ? "定义工作流启动时需要的输入。"
              : "Define the input required to start this workflow.",
          },
        },
      ],
      edges: [],
    };
    this.workflows.set(id, workflow);
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

  /** Deletes one in-memory workflow while leaving selection decisions to the UI. */
  async delete(id: string): Promise<void> {
    await waitForMockLatency();
    if (!this.workflows.delete(id)) {
      throw new Error(`Workflow "${id}" was not found`);
    }
  }

  /** Validates and imports a JSON-compatible workflow definition into the mock collection. */
  async importDefinition(value: unknown): Promise<WorkflowDefinition> {
    await waitForMockLatency();
    if (!isWorkflowDefinition(value)) {
      throw new Error("Invalid workflow definition");
    }
    const requestedId = value.id.trim();
    const id = this.workflows.has(requestedId)
      ? `${requestedId}-imported-${this.nextWorkflowId++}`
      : requestedId;
    const imported = {
      ...cloneWorkflow(value),
      id,
      updatedAt: new Date().toISOString(),
    };
    this.workflows.set(id, imported);
    return cloneWorkflow(imported);
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

/** Rejects malformed imports before they enter graph state and break canvas assumptions. */
function isWorkflowDefinition(value: unknown): value is WorkflowDefinition {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Partial<WorkflowDefinition>;
  return typeof candidate.id === "string"
    && candidate.id.trim() !== ""
    && typeof candidate.name === "string"
    && candidate.name.trim() !== ""
    && typeof candidate.description === "string"
    && typeof candidate.updatedAt === "string"
    && Array.isArray(candidate.nodes)
    && Array.isArray(candidate.edges)
    && candidate.nodes.every((node) =>
      typeof node === "object"
      && node !== null
      && typeof (node as WorkflowDefinition["nodes"][number]).id === "string"
      && ["start", "prompt", "agent", "condition", "tool", "output"].includes(
        (node as WorkflowDefinition["nodes"][number]).kind,
      )
      && typeof (node as WorkflowDefinition["nodes"][number]).title === "string"
      && typeof (node as WorkflowDefinition["nodes"][number]).description === "string"
      && typeof (node as WorkflowDefinition["nodes"][number]).position?.x === "number"
      && typeof (node as WorkflowDefinition["nodes"][number]).position?.y === "number"
      && typeof (node as WorkflowDefinition["nodes"][number]).config?.instruction === "string"
    )
    && candidate.edges.every((edge) =>
      typeof edge === "object"
      && edge !== null
      && typeof (edge as WorkflowDefinition["edges"][number]).id === "string"
      && typeof (edge as WorkflowDefinition["edges"][number]).source === "string"
      && typeof (edge as WorkflowDefinition["edges"][number]).target === "string"
    );
}

/** Keeps mock timing in one place so it can be replaced or disabled in tests later. */
function waitForMockLatency(): Promise<void> {
  return new Promise((resolve) => globalThis.setTimeout(resolve, MOCK_LATENCY_MS));
}
