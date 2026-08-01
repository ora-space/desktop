import type { Edge, Node } from "@xyflow/react";
import type { DemoWorkflow } from "./fixtures";
import type { WorkflowNodeData } from "./node-data";
import { isDemoWorkflow } from "./validation";

const MOCK_LATENCY_MS = 180;

export interface WorkflowRunResult {
  status: "success" | "failed";
  durationMs: number;
  output: string;
  steps: Array<{ nodeId: string; durationMs: number; summary: string }>;
}

/** Creates a session workflow whose graph already uses React Flow element types. */
export function createDemoWorkflow(
  id: string,
  name: string,
  locale: "zh-CN" | "en-US",
): DemoWorkflow {
  const nodes: Node<WorkflowNodeData, "workflow">[] = [
    {
      id: "start",
      type: "workflow",
      deletable: false,
      position: { x: 120, y: 260 },
      data: {
        kind: "start",
        title: locale === "zh-CN" ? "开始" : "Start",
        description: locale === "zh-CN" ? "接收工作流输入" : "Receive workflow input",
        instruction: locale === "zh-CN"
          ? "定义工作流启动时需要的输入。"
          : "Define the input required to start this workflow.",
      },
    },
  ];
  const edges: Edge[] = [];
  return {
    id,
    name,
    description: locale === "zh-CN" ? "尚未添加描述" : "No description yet",
    updatedAt: new Date().toISOString(),
    viewport: { x: 32, y: 32, zoom: 1 },
    nodes,
    edges,
  };
}

/** Parses a workflow for the current demo session without persisting it. */
export function parseDemoWorkflow(value: unknown): DemoWorkflow {
  if (!isDemoWorkflow(value)) {
    throw new Error("Invalid workflow definition");
  }
  return structuredClone(value);
}

/** Produces deterministic output from business fields stored in React Flow node data. */
export async function runDemoWorkflow(
  workflow: DemoWorkflow,
  input: string,
  locale: "zh-CN" | "en-US",
): Promise<WorkflowRunResult> {
  if (!isDemoWorkflow(workflow)) {
    throw new Error("Invalid workflow definition");
  }
  await new Promise((resolve) => globalThis.setTimeout(resolve, MOCK_LATENCY_MS));
  const steps = workflow.nodes.map((node, index) => ({
    nodeId: node.id,
    durationMs: 140 + index * 37,
    summary: locale === "zh-CN"
      ? `${node.data.title} 已完成`
      : `${node.data.title} completed`,
  }));
  const durationMs = steps.reduce((total, step) => total + step.durationMs, 0);
  return {
    status: "success",
    durationMs,
    output: locale === "zh-CN"
      ? `已完成“${workflow.name}”的模拟运行。\n\n输入：${input || "检查当前工作区的未提交改动"}\n\n发现 2 个建议项，未发现阻塞问题。`
      : `Completed a simulated run of "${workflow.name}".\n\nInput: ${input || "Review uncommitted changes in the current workspace"}\n\nFound 2 suggestions and no blocking issues.`,
    steps,
  };
}
