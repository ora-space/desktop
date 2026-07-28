import { createMockWorkflows } from "./fixtures";
import type {
  WorkflowDefinition,
  WorkflowRepository,
  WorkflowRunResult,
  WorkflowLocale,
} from "./types";

const MOCK_LATENCY_MS = 180;

const MOCK_RUN_PATHS: Record<string, readonly string[]> = {
  "code-review": ["start", "understand", "quality", "tests", "review", "output"],
  "release-readiness": [
    "release-input",
    "release-notes",
    "release-validate",
    "release-gate",
    "release-plan",
    "release-decision",
  ],
  "ci-recovery": ["ci-input", "ci-logs", "ci-classify", "ci-fix", "ci-result"],
  "dependency-update": [
    "deps-scan",
    "deps-plan",
    "deps-update",
    "deps-verify",
    "deps-output",
  ],
  "issue-triage": [
    "triage-intake",
    "triage-normalize",
    "triage-duplicates",
    "triage-severity",
    "triage-backlog",
    "triage-output",
  ],
  "openspec-change": [
    "openspec-request",
    "openspec-explore",
    "openspec-propose",
    "openspec-apply",
    "openspec-sync",
    "openspec-archive",
    "openspec-summary",
  ],
};

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

  /** Creates a blank workflow with a usable trigger so the graph is never structurally empty. */
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
          kind: "trigger",
          title: this.locale === "zh-CN" ? "手动触发" : "Manual trigger",
          description: this.locale === "zh-CN" ? "接收工作流输入" : "Receive workflow input",
          position: { x: 120, y: 260 },
          config: {
            trigger: "Manual",
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
    const runPath = MOCK_RUN_PATHS[id];
    const executedNodes = runPath === undefined
      ? workflow.nodes
      : runPath.flatMap((nodeId) => {
        const node = workflow.nodes.find((candidate) => candidate.id === nodeId);
        return node === undefined ? [] : [node];
      });
    const steps = executedNodes.map((node, index) => ({
      nodeId: node.id,
      durationMs: 140 + index * 37,
      summary: this.locale === "zh-CN" ? `${node.title} 已完成` : `${node.title} completed`,
    }));
    const durationMs = steps.reduce((total, step) => total + step.durationMs, 0);
    return {
      status: "success",
      durationMs,
      output: mockRunOutput(workflow, input, this.locale),
      steps,
    };
  }
}

/** Returns scenario-specific output so previews resemble the workflow they represent. */
function mockRunOutput(
  workflow: WorkflowDefinition,
  input: string,
  locale: WorkflowLocale,
): string {
  const zhInputs: Record<string, string> = {
    "openspec-change": "为当前需求创建并实施 OpenSpec 变更",
    "code-review": "检查当前工作区的未提交改动",
    "ci-recovery": "诊断当前 PR 最新一次失败的 CI 检查",
    "release-readiness": "检查 v0.18.0 候选版本是否可以发布",
    "issue-triage": "分诊工作流画布无法缩放的问题",
    "dependency-update": "修复当前依赖安全公告",
  };
  const enInputs: Record<string, string> = {
    "openspec-change": "Create and implement an OpenSpec change for the current request",
    "code-review": "Review uncommitted changes in the current workspace",
    "ci-recovery": "Diagnose the latest failed CI check on the current PR",
    "release-readiness": "Determine whether the v0.18.0 candidate is ready to ship",
    "issue-triage": "Triage a report that the workflow canvas cannot zoom",
    "dependency-update": "Resolve the current dependency security advisory",
  };
  const requestedInput = input || (locale === "zh-CN" ? zhInputs : enInputs)[workflow.id]
    || (locale === "zh-CN" ? "执行当前工作流" : "Run the current workflow");
  const zhOutputs: Record<string, string> = {
    "code-review": "发现 2 个建议项，未发现阻塞问题。",
    "release-readiness":
      "结论：GO\n\n候选版本 v0.18.0 的 126 项测试与生产构建均通过；数据库迁移可回滚，建议按灰度 10% → 50% → 100% 发布。",
    "issue-triage":
      "分级：P2 · 编辑器 / 工作流画布\n\n未发现完全重复的问题。已补齐复现步骤并建议分配给 Desktop UI，下一步收集 Windows 缩放比例与诊断日志。",
    "ci-recovery":
      "根因：workflow-settings 测试中的节点顺序断言已过期。\n\n已更新断言并在本地复现通过；建议重新运行失败 job。",
    "dependency-update":
      "已将受 CVE-2026-1842 影响的依赖升级到修复版本。\n\n安全扫描无高危项，锁文件仅包含目标依赖变更，完整测试通过。",
    "openspec-change":
      "变更：improve-workflow-mocks\n\nproposal、delta specs、design 与 tasks 已完成；实现通过格式化和测试，主规格已同步，变更已归档。",
  };
  const enOutputs: Record<string, string> = {
    "code-review": "Found 2 suggestions and no blocking issues.",
    "release-readiness":
      "Decision: GO\n\nAll 126 tests and the production build passed for v0.18.0. The database migration is reversible; proceed with a 10% → 50% → 100% rollout.",
    "issue-triage":
      "Priority: P2 · Editor / Workflow canvas\n\nNo exact duplicate found. Reproduction steps were normalized and ownership assigned to Desktop UI; collect Windows scaling and diagnostic logs next.",
    "ci-recovery":
      "Root cause: the workflow-settings node-order assertion was stale.\n\nThe assertion was updated and passes locally; rerun the failed job.",
    "dependency-update":
      "Updated the dependency affected by CVE-2026-1842 to its patched version.\n\nNo high-severity advisory remains, the lockfile contains only targeted changes, and the full suite passes.",
    "openspec-change":
      "Change: improve-workflow-mocks\n\nProposal, delta specs, design, and tasks are complete. Formatting and tests passed, main specs were synced, and the change was archived.",
  };
  const defaultResult = locale === "zh-CN"
    ? "模拟执行完成，所有节点均返回成功。"
    : "Simulation completed successfully for every executed node.";
  const result = (locale === "zh-CN" ? zhOutputs : enOutputs)[workflow.id] ?? defaultResult;
  return locale === "zh-CN"
    ? `已完成“${workflow.name}”的模拟运行。\n\n输入：${requestedInput}\n\n${result}`
    : `Completed a simulated run of "${workflow.name}".\n\nInput: ${requestedInput}\n\n${result}`;
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
      && [
        "trigger",
        "data-source",
        "llm",
        "code",
        "condition",
        "tool",
        "template",
        "output",
      ].includes(
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
