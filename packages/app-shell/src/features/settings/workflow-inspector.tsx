import { useState } from "react";
import {
  IconCircleCheck,
  IconClock,
  IconPlayerPlay,
  IconSettings,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import {
  Button,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Textarea,
} from "@ora/ui";
import type {
  WorkflowNode,
  WorkflowRunResult,
} from "@ora/workflow-mock";
import { getNodeMetadata } from "./workflow-node-metadata";

interface WorkflowInspectorProps {
  node: WorkflowNode | null;
  runResult: WorkflowRunResult | null;
  running: boolean;
  onUpdate: (node: WorkflowNode) => void;
  onDelete: (nodeId: string) => void;
  onCloseRun: () => void;
  onRun: (input: string) => void;
}

/** Switches between node configuration and a compact mock execution trace. */
export function WorkflowInspector(props: WorkflowInspectorProps) {
  if (props.running || props.runResult !== null) {
    return <WorkflowRunPreview {...props} />;
  }
  if (props.node === null) {
    return <WorkflowInspectorEmpty onRun={props.onRun} />;
  }
  return (
    <WorkflowNodeInspector
      node={props.node}
      onUpdate={props.onUpdate}
      onDelete={props.onDelete}
    />
  );
}

/** Guides first-time users toward selecting a node while keeping preview readily available. */
function WorkflowInspectorEmpty({ onRun }: { onRun: (input: string) => void }) {
  return (
    <aside className="flex min-h-0 flex-col border-l border-border bg-background">
      <div className="border-b border-border px-4 py-3">
        <h3 className="text-xs font-semibold">配置</h3>
        <p className="mt-1 text-[11px] text-muted-foreground">选择节点以编辑详细参数</p>
      </div>
      <div className="flex flex-1 flex-col items-center justify-center px-6 text-center">
        <span className="mb-3 flex size-10 items-center justify-center rounded-xl bg-muted">
          <IconSettings className="size-5 text-muted-foreground" />
        </span>
        <p className="text-xs font-medium">尚未选择节点</p>
        <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
          选择画布中的卡片，或直接运行一次 mock 预览。
        </p>
        <Button className="mt-4" size="sm" onClick={() => onRun("")}>
          <IconPlayerPlay />
          测试运行
        </Button>
      </div>
    </aside>
  );
}

/** Edits a node in place with visible labels and progressive, kind-specific fields. */
function WorkflowNodeInspector({
  node,
  onUpdate,
  onDelete,
}: {
  node: WorkflowNode;
  onUpdate: (node: WorkflowNode) => void;
  onDelete: (nodeId: string) => void;
}) {
  const metadata = getNodeMetadata(node.kind);
  const Icon = metadata.icon;
  return (
    <aside className="flex min-h-0 flex-col border-l border-border bg-background">
      <div className="flex items-center gap-2.5 border-b border-border px-4 py-3">
        <span className={`flex size-8 items-center justify-center rounded-lg ${metadata.tone}`}>
          <Icon className="size-4" />
        </span>
        <div>
          <h3 className="text-xs font-semibold">{node.title}</h3>
          <p className="text-[10px] text-muted-foreground">{metadata.label}节点</p>
        </div>
      </div>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
        <InspectorField label="名称" htmlFor="workflow-node-title">
          <Input
            id="workflow-node-title"
            value={node.title}
            onChange={(event) => onUpdate({ ...node, title: event.target.value })}
          />
        </InspectorField>
        <InspectorField label="说明" htmlFor="workflow-node-description">
          <Input
            id="workflow-node-description"
            value={node.description}
            onChange={(event) => onUpdate({ ...node, description: event.target.value })}
          />
        </InspectorField>
        {(node.kind === "prompt" || node.kind === "agent") && (
          <InspectorField label="模型" htmlFor="workflow-node-model">
            <Select
              value={node.config.model ?? "GPT-5"}
              onValueChange={(model) => {
                if (model !== null) {
                  onUpdate({ ...node, config: { ...node.config, model } });
                }
              }}
            >
              <SelectTrigger id="workflow-node-model" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="GPT-5">GPT-5</SelectItem>
                <SelectItem value="Claude Sonnet 4">Claude Sonnet 4</SelectItem>
                <SelectItem value="本地模型">本地模型</SelectItem>
              </SelectContent>
            </Select>
          </InspectorField>
        )}
        {node.kind === "tool" && (
          <InspectorField label="工具" htmlFor="workflow-node-tool">
            <Select
              value={node.config.tool ?? "Terminal"}
              onValueChange={(tool) => {
                if (tool !== null) {
                  onUpdate({ ...node, config: { ...node.config, tool } });
                }
              }}
            >
              <SelectTrigger id="workflow-node-tool" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="Terminal">Terminal</SelectItem>
                <SelectItem value="File system">File system</SelectItem>
                <SelectItem value="GitHub">GitHub</SelectItem>
              </SelectContent>
            </Select>
          </InspectorField>
        )}
        {node.kind === "condition" && (
          <InspectorField label="分支条件" htmlFor="workflow-node-condition">
            <Input
              id="workflow-node-condition"
              value={node.config.condition ?? ""}
              onChange={(event) =>
                onUpdate({
                  ...node,
                  config: { ...node.config, condition: event.target.value },
                })
              }
            />
          </InspectorField>
        )}
        <InspectorField label="执行指令" htmlFor="workflow-node-instruction">
          <Textarea
            id="workflow-node-instruction"
            className="min-h-32 resize-none text-xs leading-5"
            value={node.config.instruction}
            onChange={(event) =>
              onUpdate({
                ...node,
                config: { ...node.config, instruction: event.target.value },
              })
            }
          />
        </InspectorField>
      </div>
      <div className="border-t border-border p-3">
        <Button
          variant="ghost"
          className="w-full justify-start text-destructive hover:bg-destructive/10 hover:text-destructive"
          onClick={() => onDelete(node.id)}
        >
          <IconTrash />
          删除节点
        </Button>
      </div>
    </aside>
  );
}

/** Displays deterministic mock progress and output without implying a real agent was executed. */
function WorkflowRunPreview({
  running,
  runResult,
  onCloseRun,
  onRun,
}: WorkflowInspectorProps) {
  const [input, setInput] = useState("检查当前工作区的未提交改动");

  return (
    <aside className="flex min-h-0 flex-col border-l border-border bg-background" aria-live="polite">
      <div className="flex items-center justify-between border-b border-border px-4 py-3">
        <div>
          <h3 className="text-xs font-semibold">测试运行</h3>
          <p className="mt-0.5 text-[10px] text-muted-foreground">仅使用 mock 数据，不会执行真实工具</p>
        </div>
        <Button variant="ghost" size="icon-sm" aria-label="关闭测试结果" onClick={onCloseRun}>
          <IconX />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {running ? (
          <div className="flex h-full flex-col items-center justify-center text-center">
            <span className="relative mb-3 flex size-11 items-center justify-center rounded-full bg-primary/10">
              <IconPlayerPlay className="size-5 animate-pulse" />
            </span>
            <p className="text-xs font-medium">正在模拟工作流…</p>
            <p className="mt-1 text-[11px] text-muted-foreground">逐步执行节点并收集输出</p>
          </div>
        ) : runResult !== null ? (
          <div className="space-y-4">
            <div className="flex items-center gap-2 rounded-lg border border-emerald-500/25 bg-emerald-500/8 p-3">
              <IconCircleCheck className="size-4 text-emerald-600 dark:text-emerald-400" />
              <div>
                <p className="text-xs font-medium">模拟运行成功</p>
                <p className="text-[10px] text-muted-foreground">{runResult.durationMs} ms</p>
              </div>
            </div>
            <div>
              <p className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">执行轨迹</p>
              <ol className="space-y-2">
                {runResult.steps.map((step) => (
                  <li key={step.nodeId} className="flex items-center gap-2 text-[11px]">
                    <IconCircleCheck className="size-3.5 text-emerald-600 dark:text-emerald-400" />
                    <span className="min-w-0 flex-1 truncate">{step.summary}</span>
                    <span className="flex items-center gap-1 tabular-nums text-muted-foreground">
                      <IconClock className="size-3" />
                      {step.durationMs} ms
                    </span>
                  </li>
                ))}
              </ol>
            </div>
            <div>
              <p className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">输出</p>
              <pre data-selectable className="whitespace-pre-wrap rounded-lg bg-muted/70 p-3 font-sans text-[11px] leading-5">
                {runResult.output}
              </pre>
            </div>
          </div>
        ) : null}
      </div>
      <div className="space-y-2 border-t border-border p-3">
        <Label htmlFor="workflow-preview-input" className="text-[10px]">测试输入</Label>
        <Textarea
          id="workflow-preview-input"
          value={input}
          onChange={(event) => setInput(event.target.value)}
          className="min-h-16 resize-none text-xs"
        />
        <Button className="w-full" size="sm" disabled={running} onClick={() => onRun(input)}>
          <IconPlayerPlay />
          再次运行
        </Button>
      </div>
    </aside>
  );
}

/** Keeps field labels visible and consistently spaced for scanning and accessibility. */
function InspectorField({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={htmlFor} className="text-[11px]">{label}</Label>
      {children}
    </div>
  );
}
