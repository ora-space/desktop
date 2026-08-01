import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconCircleCheck,
  IconCircleX,
  IconClock,
  IconLayoutSidebarRightCollapse,
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
import {
  type WorkflowNodeData,
  type WorkflowCapabilities,
  type WorkflowRunResult,
} from "@ora/workflow-mock";
import type { Node } from "@xyflow/react";
import { getNodeMetadata } from "./workflow-node-metadata";

interface WorkflowInspectorProps {
  node: Node<WorkflowNodeData, "workflow"> | null;
  runResult: WorkflowRunResult | null;
  running: boolean;
  capabilities: WorkflowCapabilities;
  onUpdate: (node: Node<WorkflowNodeData, "workflow">) => void;
  onDelete: (nodeId: string) => void;
  onCloseRun: () => void;
  onCloseNode: () => void;
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
      capabilities={props.capabilities}
      onUpdate={props.onUpdate}
      onDelete={props.onDelete}
      onClose={props.onCloseNode}
    />
  );
}

/** Guides first-time users toward selecting a node while keeping preview readily available. */
function WorkflowInspectorEmpty({ onRun }: { onRun: (input: string) => void }) {
  const { t } = useTranslation();
  return (
    <aside className="flex min-h-0 flex-1 flex-col border-l border-border bg-background">
      <div className="border-b border-border px-4 py-3">
        <h3 className="text-xs font-semibold">{t("settings.workflow.configuration")}</h3>
        <p className="mt-1 text-[11px] text-muted-foreground">{t("settings.workflow.selectNodeHint")}</p>
      </div>
      <div className="flex flex-1 flex-col items-center justify-center px-6 text-center">
        <span className="mb-3 flex size-10 items-center justify-center rounded-xl bg-muted">
          <IconSettings className="size-5 text-muted-foreground" />
        </span>
        <p className="text-xs font-medium">{t("settings.workflow.noSelection")}</p>
        <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
          {t("settings.workflow.noSelectionHint")}
        </p>
        <Button className="mt-4" size="sm" onClick={() => onRun("")}>
          <IconPlayerPlay />
          {t("settings.workflow.testRun")}
        </Button>
      </div>
    </aside>
  );
}

/** Edits a node in place with visible labels and progressive, kind-specific fields. */
function WorkflowNodeInspector({
  node,
  capabilities,
  onUpdate,
  onDelete,
  onClose,
}: {
  node: Node<WorkflowNodeData, "workflow">;
  capabilities: WorkflowCapabilities;
  onUpdate: (node: Node<WorkflowNodeData, "workflow">) => void;
  onDelete: (nodeId: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const metadata = getNodeMetadata(node.data.kind);
  const nodeType = capabilities.nodeTypes.find((candidate) => candidate.kind === node.data.kind);
  if (nodeType === undefined) {
    throw new Error(`Missing workflow capability for node kind "${node.data.kind}"`);
  }
  const Icon = metadata.icon;
  return (
    <aside className="flex min-h-0 flex-1 flex-col border-l border-border bg-background">
      <div className="flex items-center gap-2.5 border-b border-border px-4 py-3">
        <span className={`flex size-8 items-center justify-center rounded-lg ${metadata.tone}`}>
          <Icon className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="text-xs font-semibold">{node.data.title}</h3>
          <p className="text-[10px] text-muted-foreground">
            {t("settings.workflow.nodeSuffix", { type: nodeType.label })}
          </p>
        </div>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("settings.workflow.closeConfiguration")}
          onClick={onClose}
        >
          <IconLayoutSidebarRightCollapse />
        </Button>
      </div>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
        <InspectorField label={t("settings.workflow.field.name")} htmlFor="workflow-node-title">
          <Input
            id="workflow-node-title"
            value={node.data.title}
            onChange={(event) => onUpdate({
              ...node,
              data: { ...node.data, title: event.target.value },
            })}
          />
        </InspectorField>
        <InspectorField label={t("settings.workflow.field.description")} htmlFor="workflow-node-description">
          <Input
            id="workflow-node-description"
            value={node.data.description}
            onChange={(event) => onUpdate({
              ...node,
              data: { ...node.data, description: event.target.value },
            })}
          />
        </InspectorField>
        {nodeType.configFields.includes("model") && (
          <InspectorField label={t("settings.workflow.field.model")} htmlFor="workflow-node-model">
            <Select
              value={node.data.model ?? capabilities.defaultModel}
              onValueChange={(model) => {
                if (model !== null) {
                  onUpdate({ ...node, data: { ...node.data, model } });
                }
              }}
            >
              <SelectTrigger id="workflow-node-model" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {capabilities.models.map((model) => (
                  <SelectItem key={model.value} value={model.value}>
                    {model.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </InspectorField>
        )}
        {nodeType.configFields.includes("tool") && (
          <InspectorField label={t("settings.workflow.field.tool")} htmlFor="workflow-node-tool">
            <Select
              value={node.data.tool ?? capabilities.defaultTool}
              onValueChange={(tool) => {
                if (tool !== null) {
                  onUpdate({ ...node, data: { ...node.data, tool } });
                }
              }}
            >
              <SelectTrigger id="workflow-node-tool" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {capabilities.tools.map((tool) => (
                  <SelectItem key={tool.value} value={tool.value}>
                    {tool.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </InspectorField>
        )}
        {nodeType.configFields.includes("condition") && (
          <InspectorField label={t("settings.workflow.field.condition")} htmlFor="workflow-node-condition">
            <Input
              id="workflow-node-condition"
              value={node.data.condition ?? ""}
              onChange={(event) =>
                onUpdate({
                  ...node,
                  data: { ...node.data, condition: event.target.value },
                })
              }
            />
          </InspectorField>
        )}
        {nodeType.configFields.includes("instruction") && (
          <InspectorField label={t("settings.workflow.field.instruction")} htmlFor="workflow-node-instruction">
            <Textarea
              id="workflow-node-instruction"
              className="min-h-32 resize-none text-xs leading-5"
              value={node.data.instruction}
              onChange={(event) =>
                onUpdate({
                  ...node,
                  data: { ...node.data, instruction: event.target.value },
                })
              }
            />
          </InspectorField>
        )}
      </div>
      <div className="border-t border-border p-3">
        <Button
          variant="ghost"
          className="w-full justify-start text-destructive hover:bg-destructive/10 hover:text-destructive"
          onClick={() => onDelete(node.id)}
          disabled={node.data.kind === "start"}
        >
          <IconTrash />
          {t("settings.workflow.deleteNode")}
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
  const { t } = useTranslation();
  const [input, setInput] = useState(() => t("settings.workflow.previewInput"));
  const succeeded = runResult?.status !== "failed";
  const ResultIcon = succeeded ? IconCircleCheck : IconCircleX;

  return (
    <aside className="flex min-h-0 flex-1 flex-col border-l border-border bg-background" aria-live="polite">
      <div className="flex items-center justify-between border-b border-border px-4 py-3">
        <div>
          <h3 className="text-xs font-semibold">{t("settings.workflow.testRun")}</h3>
          <p className="mt-0.5 text-[10px] text-muted-foreground">{t("settings.workflow.mockNotice")}</p>
        </div>
        <Button variant="ghost" size="icon-sm" aria-label={t("settings.workflow.closePreview")} onClick={onCloseRun}>
          <IconX />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {running ? (
          <div className="flex h-full flex-col items-center justify-center text-center">
            <span className="relative mb-3 flex size-11 items-center justify-center rounded-full bg-primary/10">
              <IconPlayerPlay className="size-5 animate-pulse" />
            </span>
            <p className="text-xs font-medium">{t("settings.workflow.running")}</p>
            <p className="mt-1 text-[11px] text-muted-foreground">{t("settings.workflow.runningHint")}</p>
          </div>
        ) : runResult !== null ? (
          <div className="space-y-4">
            <div
              className={`flex items-center gap-2 rounded-lg border p-3 ${
                succeeded
                  ? "border-emerald-500/25 bg-emerald-500/8"
                  : "border-destructive/25 bg-destructive/8"
              }`}
            >
              <ResultIcon
                className={`size-4 ${
                  succeeded
                    ? "text-emerald-600 dark:text-emerald-400"
                    : "text-destructive"
                }`}
              />
              <div>
                <p className="text-xs font-medium">
                  {t(
                    succeeded
                      ? "settings.workflow.runSuccess"
                      : "settings.workflow.runFailed",
                  )}
                </p>
                <p className="text-[10px] text-muted-foreground">{runResult.durationMs} ms</p>
              </div>
            </div>
            <div>
              <p className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">{t("settings.workflow.trace")}</p>
              <ol className="space-y-2">
                {runResult.steps.map((step) => (
                  <li key={step.nodeId} className="flex items-center gap-2 text-[11px]">
                    <ResultIcon
                      className={`size-3.5 ${
                        succeeded
                          ? "text-emerald-600 dark:text-emerald-400"
                          : "text-destructive"
                      }`}
                    />
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
              <p className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">{t("settings.workflow.output")}</p>
              <pre data-selectable className="whitespace-pre-wrap rounded-lg bg-muted/70 p-3 font-sans text-[11px] leading-5">
                {runResult.output}
              </pre>
            </div>
          </div>
        ) : null}
      </div>
      <div className="space-y-2 border-t border-border p-3">
        <Label htmlFor="workflow-preview-input" className="text-[10px]">{t("settings.workflow.testInput")}</Label>
        <Textarea
          id="workflow-preview-input"
          value={input}
          onChange={(event) => setInput(event.target.value)}
          className="min-h-16 resize-none text-xs"
        />
        <Button className="w-full" size="sm" disabled={running} onClick={() => onRun(input)}>
          <IconPlayerPlay />
          {t("settings.workflow.runAgain")}
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
