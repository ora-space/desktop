import type { Edge, Node } from "@xyflow/react";
import type { DemoWorkflow } from "./fixtures";
import type { WorkflowNodeData } from "./node-data";
import { isDemoWorkflow } from "./validation";

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
