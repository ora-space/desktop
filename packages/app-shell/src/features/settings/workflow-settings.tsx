import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconCheck,
  IconCloudCheck,
  IconPlayerPlay,
  IconRoute,
} from "@tabler/icons-react";
import { Button, Input, Skeleton } from "@ora/ui";
import {
  MockWorkflowRepository,
  type WorkflowDefinition,
  type WorkflowLocale,
  type WorkflowNode,
  type WorkflowNodeKind,
  type WorkflowRunResult,
} from "@ora/workflow-mock";
import { WorkflowCanvas } from "./workflow-canvas";
import { WorkflowInspector } from "./workflow-inspector";
import { WorkflowManager } from "./workflow-manager";
import { WorkflowNodeCatalog } from "./workflow-node-catalog";
import { getNodeMetadata } from "./workflow-node-metadata";

/** Owns the frontend-only workflow editor state and coordinates the mock repository boundary. */
export function WorkflowSettings() {
  const { i18n, t } = useTranslation();
  const locale: WorkflowLocale = i18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN";
  const repository = useMemo(() => new MockWorkflowRepository(locale), [locale]);
  const [workflows, setWorkflows] = useState<WorkflowDefinition[]>([]);
  const [selectedWorkflowId, setSelectedWorkflowId] = useState<string | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [managing, setManaging] = useState(false);
  const [managerError, setManagerError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [dirtyWorkflowIds, setDirtyWorkflowIds] = useState<Set<string>>(() => new Set());
  const [running, setRunning] = useState(false);
  const [runResult, setRunResult] = useState<WorkflowRunResult | null>(null);
  const nextNodeNumber = useRef(1);
  const nextEdgeNumber = useRef(1);
  const workflow = useMemo(
    () => workflows.find((candidate) => candidate.id === selectedWorkflowId) ?? null,
    [selectedWorkflowId, workflows],
  );
  const saved = selectedWorkflowId === null || !dirtyWorkflowIds.has(selectedWorkflowId);

  useEffect(() => {
    let active = true;
    setLoading(true);
    repository.list().then((loaded) => {
      if (active) {
        setWorkflows(loaded);
        setSelectedWorkflowId(loaded[0]?.id ?? null);
        setDirtyWorkflowIds(new Set());
        setLoading(false);
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

  /** Applies one graph or metadata mutation while keeping dirty-state behavior consistent. */
  function updateWorkflow(
    updater: (current: WorkflowDefinition) => WorkflowDefinition,
  ): void {
    setWorkflows((current) =>
      current.map((candidate) =>
        candidate.id === selectedWorkflowId ? updater(candidate) : candidate,
      ),
    );
    if (selectedWorkflowId !== null) {
      setDirtyWorkflowIds((current) => new Set(current).add(selectedWorkflowId));
    }
  }

  /** Switches the active graph and clears transient state that belongs to the previous workflow. */
  function selectWorkflow(workflowId: string): void {
    setSelectedWorkflowId(workflowId);
    setSelectedNodeId(null);
    setRunResult(null);
    setManagerError(null);
  }

  /** Creates a usable blank workflow and immediately opens it for editing. */
  async function createWorkflow(): Promise<void> {
    setManaging(true);
    setManagerError(null);
    try {
      const created = await repository.create(
        t("settings.workflow.untitledWorkflow", { count: workflows.length + 1 }),
      );
      setWorkflows((current) => [...current, created]);
      selectWorkflow(created.id);
    } catch {
      setManagerError(t("settings.workflow.manageError"));
    } finally {
      setManaging(false);
    }
  }

  /** Deletes a workflow and selects the nearest remaining item to avoid a dead editor state. */
  async function deleteWorkflow(workflowId: string): Promise<void> {
    setManaging(true);
    setManagerError(null);
    try {
      await repository.delete(workflowId);
      const remaining = workflows.filter((candidate) => candidate.id !== workflowId);
      setWorkflows(remaining);
      if (selectedWorkflowId === workflowId) {
        setSelectedWorkflowId(remaining[0]?.id ?? null);
        setSelectedNodeId(null);
        setRunResult(null);
      }
      setDirtyWorkflowIds((current) => {
        const next = new Set(current);
        next.delete(workflowId);
        return next;
      });
    } catch {
      setManagerError(t("settings.workflow.manageError"));
    } finally {
      setManaging(false);
    }
  }

  /** Parses and validates an exported workflow through the mock repository before selection. */
  async function importWorkflow(file: File): Promise<void> {
    setManaging(true);
    setManagerError(null);
    try {
      const imported = await repository.importDefinition(JSON.parse(await file.text()));
      setWorkflows((current) => [...current, imported]);
      selectWorkflow(imported.id);
    } catch {
      setManagerError(t("settings.workflow.importError"));
    } finally {
      setManaging(false);
    }
  }

  /** Adds a catalog node at a canvas-provided position and selects it for immediate editing. */
  function addNode(kind: WorkflowNodeKind, position: WorkflowNode["position"]): void {
    const metadata = getNodeMetadata(kind);
    const sequence = nextNodeNumber.current++;
    const id = `${kind}-${sequence}`;
    const node: WorkflowNode = {
      id,
      kind,
      title: `${t(metadata.labelKey)} ${sequence}`,
      description: t(metadata.descriptionKey),
      position,
      config: {
        instruction: "",
        ...(kind === "trigger" ? { trigger: "Manual" } : {}),
        ...(kind === "data-source" ? { source: "Workspace" } : {}),
        ...(kind === "llm" ? { model: "GPT-5" } : {}),
        ...(kind === "code" ? { language: "Shell", command: "" } : {}),
        ...(kind === "tool" ? { tool: "Terminal" } : {}),
        ...(kind === "tool" ? { command: "" } : {}),
        ...(kind === "condition"
          ? { condition: t("settings.workflow.defaultCondition") }
          : {}),
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
      edges: current.edges.filter(
        (edge) => edge.source !== nodeId && edge.target !== nodeId,
      ),
    }));
    setSelectedNodeId((current) => (current === nodeId ? null : current));
  }

  /** Removes one connection without affecting either endpoint node. */
  function deleteEdge(edgeId: string): void {
    updateWorkflow((current) => ({
      ...current,
      edges: current.edges.filter((edge) => edge.id !== edgeId),
    }));
  }

  /** Creates a unique directed edge and ignores duplicate links. */
  function connectNodes(source: string, target: string): void {
    updateWorkflow((current) => {
      if (
        current.edges.some((edge) => edge.source === source && edge.target === target)
      ) {
        return current;
      }
      return {
        ...current,
        edges: [
          ...current.edges,
          {
            id: `edge-${source}-${target}-${nextEdgeNumber.current++}`,
            source,
            target,
          },
        ],
      };
    });
  }

  /** Moves either edge endpoint while rejecting self-links and duplicate connections. */
  function reconnectEdge(edgeId: string, source: string, target: string): void {
    updateWorkflow((current) => {
      if (
        source === target
        || current.edges.some(
          (edge) =>
            edge.id !== edgeId
            && edge.source === source
            && edge.target === target,
        )
      ) {
        return current;
      }
      return {
        ...current,
        edges: current.edges.map((edge) =>
          edge.id === edgeId ? { ...edge, source, target } : edge,
        ),
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
      setWorkflows((current) =>
        current.map((candidate) => candidate.id === persisted.id ? persisted : candidate),
      );
      setDirtyWorkflowIds((current) => {
        const next = new Set(current);
        next.delete(persisted.id);
        return next;
      });
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

  if (loading) {
    return <WorkflowLoading />;
  }

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      <header className="flex min-h-14 items-center gap-3 border-b border-border py-2 pl-3 pr-12 sm:pl-4">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-foreground text-background">
          <IconRoute className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          {workflow === null ? (
            <h2 className="text-sm font-semibold">{t("settings.workflow.library")}</h2>
          ) : (
            <>
              <div className="flex items-center gap-2">
                <Input
                  value={workflow.name}
                  onChange={(event) =>
                    updateWorkflow((current) => ({ ...current, name: event.target.value }))
                  }
                  aria-label={t("settings.workflow.workflowName")}
                  className="h-7 max-w-72 border-transparent bg-transparent px-1 text-sm font-semibold shadow-none hover:border-border focus-visible:border-border"
                />
                <span className="hidden rounded-full border border-border px-2 py-0.5 text-[9px] font-medium text-muted-foreground sm:inline">
                  MOCK
                </span>
              </div>
              <p className="truncate px-1 text-[10px] text-muted-foreground">
                {workflow.description}
              </p>
            </>
          )}
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => void runWorkflow("")}
          disabled={workflow === null || running}
        >
          <IconPlayerPlay />
          <span className="hidden sm:inline">{t("settings.workflow.testRun")}</span>
        </Button>
        <Button
          size="sm"
          onClick={() => void saveWorkflow()}
          disabled={workflow === null || saving || saved}
        >
          {saved ? <IconCheck /> : <IconCloudCheck />}
          <span className="hidden sm:inline">
            {saving
              ? t("common.saving")
              : saved
                ? t("settings.workflow.saved")
                : t("common.save")}
          </span>
        </Button>
      </header>
      <div className="grid min-h-0 flex-1 grid-cols-[200px_minmax(0,1fr)_260px] xl:grid-cols-[220px_minmax(0,1fr)_280px]">
        <WorkflowManager
          workflows={workflows}
          selectedWorkflowId={selectedWorkflowId}
          busy={managing}
          error={managerError}
          onSelect={selectWorkflow}
          onCreate={() => void createWorkflow()}
          onDelete={(workflowId) => void deleteWorkflow(workflowId)}
          onImport={(file) => void importWorkflow(file)}
        />
        {workflow === null ? (
          <WorkflowEmpty onCreate={() => void createWorkflow()} />
        ) : (
          <WorkflowCanvas
            key={workflow.id}
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
            onAddNode={addNode}
            onConnect={connectNodes}
            onReconnectEdge={reconnectEdge}
            onDeleteNode={deleteNode}
            onDeleteEdge={deleteEdge}
          >
            {(onAddNode) => <WorkflowNodeCatalog onAdd={onAddNode} />}
          </WorkflowCanvas>
        )}
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

/** Gives an empty collection a clear recovery action without disguising it as a loading state. */
function WorkflowEmpty({ onCreate }: { onCreate: () => void }) {
  const { t } = useTranslation();
  return (
    <section className="flex min-h-0 items-center justify-center bg-muted/25">
      <div className="max-w-64 text-center">
        <span className="mx-auto flex size-10 items-center justify-center rounded-xl border border-border bg-background shadow-sm">
          <IconRoute className="size-4 text-muted-foreground" />
        </span>
        <h3 className="mt-3 text-sm font-semibold">{t("settings.workflow.emptyTitle")}</h3>
        <p className="mt-1 text-[11px] leading-4 text-muted-foreground">
          {t("settings.workflow.emptyDescription")}
        </p>
        <Button size="sm" className="mt-4" onClick={onCreate}>
          {t("settings.workflow.newWorkflow")}
        </Button>
      </div>
    </section>
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
