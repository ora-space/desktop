import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconCheck,
  IconChevronDown,
  IconCloudCheck,
  IconPlayerPlay,
  IconPlus,
  IconRoute,
} from "@tabler/icons-react";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Skeleton,
} from "@ora/ui";
import {
  MockWorkflowRepository,
  type WorkflowDefinition,
  type WorkflowNode,
  type WorkflowNodeKind,
  type WorkflowLocale,
  type WorkflowRunResult,
} from "@ora/workflow-mock";
import { WorkflowCanvas } from "./workflow-canvas";
import {
  WorkflowNodeCatalog,
} from "./workflow-node-catalog";
import {
  WORKFLOW_NODE_CATALOG,
  getNodeMetadata,
} from "./workflow-node-metadata";
import { WorkflowInspector } from "./workflow-inspector";

/** Owns the frontend-only workflow editor state and coordinates the mock repository boundary. */
export function WorkflowSettings() {
  const { i18n, t } = useTranslation();
  const locale: WorkflowLocale = i18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN";
  const repository = useMemo(() => new MockWorkflowRepository(locale), [locale]);
  const [workflow, setWorkflow] = useState<WorkflowDefinition | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(true);
  const [running, setRunning] = useState(false);
  const [runResult, setRunResult] = useState<WorkflowRunResult | null>(null);
  const nextNodeNumber = useRef(1);
  const nextEdgeNumber = useRef(1);

  useEffect(() => {
    let active = true;
    repository.get("code-review").then((loaded) => {
      if (active) {
        setWorkflow(loaded);
      }
    });
    return () => {
      active = false;
    };
  }, [repository]);

  const selectedNode = useMemo(
    () => workflow?.nodes.find((node) => node.id === selectedNodeId) ?? null,
    [selectedNodeId, workflow],
  );

  /** Applies one graph mutation while keeping dirty-state behavior consistent. */
  function updateWorkflow(
    updater: (current: WorkflowDefinition) => WorkflowDefinition,
  ): void {
    setWorkflow((current) => current === null ? null : updater(current));
    setSaved(false);
  }

  /** Adds a catalog node at a staggered central position so repeated additions stay visible. */
  function addNode(kind: WorkflowNodeKind): void {
    const metadata = getNodeMetadata(kind);
    const sequence = nextNodeNumber.current++;
    const id = `${kind}-${sequence}`;
    const node: WorkflowNode = {
      id,
      kind,
      title: `${t(metadata.labelKey)} ${sequence}`,
      description: t(metadata.descriptionKey),
      position: {
        x: 430 + (sequence % 3) * 42,
        y: 250 + (sequence % 4) * 38,
      },
      config: {
        instruction: "",
        ...(kind === "prompt" || kind === "agent" ? { model: "GPT-5" } : {}),
        ...(kind === "tool" ? { tool: "Terminal" } : {}),
        ...(kind === "condition" ? { condition: t("settings.workflow.defaultCondition") } : {}),
      },
    };
    updateWorkflow((current) => ({ ...current, nodes: [...current.nodes, node] }));
    setSelectedNodeId(id);
  }

  /** Removes a node and all incident edges so dangling graph references are impossible. */
  function deleteNode(nodeId: string): void {
    updateWorkflow((current) => ({
      ...current,
      nodes: current.nodes.filter((node) => node.id !== nodeId),
      edges: current.edges.filter((edge) => edge.source !== nodeId && edge.target !== nodeId),
    }));
    setSelectedNodeId((current) => current === nodeId ? null : current);
  }

  /** Creates a unique directed edge and ignores duplicate links. */
  function connectNodes(source: string, target: string): void {
    updateWorkflow((current) => {
      if (current.edges.some((edge) => edge.source === source && edge.target === target)) {
        return current;
      }
      return {
        ...current,
        edges: [
          ...current.edges,
          { id: `edge-${source}-${target}-${nextEdgeNumber.current++}`, source, target },
        ],
      };
    });
  }

  /** Saves through the mock boundary so switching to a backend only replaces the repository. */
  async function saveWorkflow(): Promise<void> {
    if (workflow === null) {
      return;
    }
    setSaving(true);
    try {
      const persisted = await repository.save(workflow);
      setWorkflow(persisted);
      setSaved(true);
    } finally {
      setSaving(false);
    }
  }

  /** Runs the deterministic mock preview and exposes progress before showing its trace. */
  async function runWorkflow(input: string): Promise<void> {
    if (workflow === null) {
      return;
    }
    setRunning(true);
    setRunResult(null);
    try {
      setRunResult(await repository.run(workflow.id, input));
    } finally {
      setRunning(false);
    }
  }

  if (workflow === null) {
    return <WorkflowLoading />;
  }

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      <header className="flex min-h-14 items-center gap-3 border-b border-border py-2 pl-3 pr-12 sm:pl-4">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-foreground text-background">
          <IconRoute className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h2 className="truncate text-sm font-semibold">{workflow.name}</h2>
            <span className="hidden rounded-full border border-border px-2 py-0.5 text-[9px] font-medium text-muted-foreground sm:inline">
              MOCK
            </span>
          </div>
          <p className="truncate text-[10px] text-muted-foreground">{workflow.description}</p>
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button variant="outline" size="sm" className="lg:hidden">
                <IconPlus />
                {t("settings.workflow.add")}
                <IconChevronDown />
              </Button>
            }
          >
            <span className="sr-only">{t("settings.workflow.addNode")}</span>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {WORKFLOW_NODE_CATALOG.map((item) => {
              const Icon = item.icon;
              return (
                <DropdownMenuItem key={item.kind} onClick={() => addNode(item.kind)}>
                  <Icon />
                  {t(item.labelKey)}
                </DropdownMenuItem>
              );
            })}
          </DropdownMenuContent>
        </DropdownMenu>
        <Button
          variant="outline"
          size="sm"
          onClick={() => void runWorkflow("")}
          disabled={running}
        >
          <IconPlayerPlay />
          <span className="hidden sm:inline">{t("settings.workflow.testRun")}</span>
        </Button>
        <Button size="sm" onClick={() => void saveWorkflow()} disabled={saving || saved}>
          {saved ? <IconCheck /> : <IconCloudCheck />}
          <span className="hidden sm:inline">
            {saving ? t("common.saving") : saved ? t("settings.workflow.saved") : t("common.save")}
          </span>
        </Button>
      </header>
      <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_260px] lg:grid-cols-[176px_minmax(0,1fr)_280px]">
        <WorkflowNodeCatalog onAdd={addNode} />
        <WorkflowCanvas
          nodes={workflow.nodes}
          edges={workflow.edges}
          selectedNodeId={selectedNodeId}
          onSelectNode={setSelectedNodeId}
          onMoveNode={(nodeId, position) =>
            updateWorkflow((current) => ({
              ...current,
              nodes: current.nodes.map((node) =>
                node.id === nodeId ? { ...node, position } : node,
              ),
            }))
          }
          onConnect={connectNodes}
          onDeleteNode={deleteNode}
        />
        <WorkflowInspector
          node={selectedNode}
          running={running}
          runResult={runResult}
          onUpdate={(updatedNode) =>
            updateWorkflow((current) => ({
              ...current,
              nodes: current.nodes.map((node) =>
                node.id === updatedNode.id ? updatedNode : node,
              ),
            }))
          }
          onDelete={deleteNode}
          onCloseRun={() => setRunResult(null)}
          onRun={(input) => void runWorkflow(input)}
        />
      </div>
    </div>
  );
}

/** Reserves the final editor layout while mock data loads to prevent a visible layout jump. */
function WorkflowLoading() {
  const { t } = useTranslation();
  return (
    <div className="flex h-full flex-col">
      <div className="flex h-14 items-center gap-3 border-b border-border px-4">
        <Skeleton className="size-8 rounded-lg" />
        <div className="space-y-1.5">
          <Skeleton className="h-3 w-40" />
          <Skeleton className="h-2.5 w-72" />
        </div>
      </div>
      <div className="flex flex-1 items-center justify-center">
        <p className="text-xs text-muted-foreground">{t("settings.workflow.loading")}</p>
      </div>
    </div>
  );
}
