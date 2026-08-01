import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  addEdge,
  applyEdgeChanges,
  applyNodeChanges,
  ReactFlowProvider,
  reconnectEdge as reconnectReactFlowEdge,
  useReactFlow,
  type Connection,
  type Edge,
  type EdgeChange,
  type Node,
  type NodeChange,
  type Viewport,
  type XYPosition,
} from "@xyflow/react";
import {
  IconPlayerPlay,
  IconRoute,
} from "@tabler/icons-react";
import {
  Button,
  Input,
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  type ResizablePanelHandle,
} from "@ora/ui";
import {
  createMockWorkflowCapabilities,
  createDemoWorkflow,
  createMockWorkflowNode,
  createMockWorkflows,
  parseDemoWorkflow,
  runDemoWorkflow,
  type DemoWorkflow,
  type WorkflowCapabilities,
  type WorkflowNodeData,
  type WorkflowNodeKind,
  type WorkflowRunResult,
} from "@ora/workflow-mock";
import { WorkflowCanvas } from "./workflow-canvas";
import { WorkflowInspector } from "./workflow-inspector";
import { WorkflowManager } from "./workflow-manager";
import {
  animateWorkflowPanel,
  cancelWorkflowPanelAnimation,
} from "./workflow-panel-motion";

const DEFAULT_WORKFLOW_LIBRARY_WIDTH = 220;
const MIN_WORKFLOW_LIBRARY_WIDTH = 180;
const MAX_WORKFLOW_LIBRARY_WIDTH = 320;
const WORKFLOW_LIBRARY_COLLAPSE_THRESHOLD = 130;
const WORKFLOW_LIBRARY_FADE_START = 90;
const DEFAULT_WORKFLOW_INSPECTOR_WIDTH = 320;
const MIN_WORKFLOW_INSPECTOR_WIDTH = 240;
const MAX_WORKFLOW_INSPECTOR_WIDTH = 480;
const WORKFLOW_INSPECTOR_COLLAPSE_THRESHOLD = 180;
const WORKFLOW_INSPECTOR_FADE_START = 120;
const WORKFLOW_PANEL_SETTLE_DURATION = 180;
const MIN_WORKFLOW_CANVAS_WIDTH = 360;
const NARROW_WORKFLOW_EDITOR_WIDTH = 1_000;

export interface WorkflowSettingsProps {
  capabilities?: WorkflowCapabilities;
}

/** Finds the first readable graph ID that does not collide with session elements. */
function uniqueGraphId(prefix: string, existingIds: Iterable<string>): {
  id: string;
  sequence: number;
} {
  const existing = new Set(existingIds);
  let sequence = 1;
  while (existing.has(`${prefix}-${sequence}`)) {
    sequence += 1;
  }
  return { id: `${prefix}-${sequence}`, sequence };
}

/** Provides one React Flow store to the canvas and its sibling inspector. */
export function WorkflowSettings(props: WorkflowSettingsProps = {}) {
  return (
    <ReactFlowProvider>
      <WorkflowSettingsContent {...props} />
    </ReactFlowProvider>
  );
}

/** Owns the complete workflow demo as disposable, session-only React state. */
function WorkflowSettingsContent({
  capabilities: capabilitiesOverride,
}: WorkflowSettingsProps) {
  const { i18n, t } = useTranslation();
  const { deleteElements } = useReactFlow();
  const locale = i18n.resolvedLanguage === "en-US" ? "en-US" as const : "zh-CN" as const;
  const capabilities = useMemo(
    () => capabilitiesOverride ?? createMockWorkflowCapabilities(locale),
    [capabilitiesOverride, locale],
  );
  const [workflows, setWorkflows] = useState<DemoWorkflow[]>(() =>
    createMockWorkflows(locale),
  );
  const [selectedWorkflowId, setSelectedWorkflowId] = useState<string | null>("code-review");
  const [managerError, setManagerError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [runResult, setRunResult] = useState<WorkflowRunResult | null>(null);
  const runRequestRef = useRef(0);
  const editorLayoutRef = useRef<HTMLDivElement>(null);
  const libraryPanelRef = useRef<ResizablePanelHandle | null>(null);
  const inspectorPanelRef = useRef<ResizablePanelHandle | null>(null);
  const libraryAnimationRef = useRef<number | null>(null);
  const inspectorAnimationRef = useRef<number | null>(null);
  const initialLibraryWidth = DEFAULT_WORKFLOW_LIBRARY_WIDTH;
  const initialInspectorWidth = DEFAULT_WORKFLOW_INSPECTOR_WIDTH;
  const libraryWidthRef = useRef(initialLibraryWidth);
  const inspectorWidthRef = useRef(initialInspectorWidth);
  const libraryCurrentWidthRef = useRef(initialLibraryWidth);
  const inspectorCurrentWidthRef = useRef(0);
  const [libraryCollapsed, setLibraryCollapsed] = useState(false);
  const [inspectorCollapsed, setInspectorCollapsed] = useState(true);
  const [libraryVisualWidth, setLibraryVisualWidth] = useState(initialLibraryWidth);
  const [inspectorVisualWidth, setInspectorVisualWidth] = useState(0);
  const workflow = useMemo(
    () => workflows.find((candidate) => candidate.id === selectedWorkflowId) ?? null,
    [selectedWorkflowId, workflows],
  );
  useEffect(() => {
    if (
      workflows.length > 0
      && !workflows.some((candidate) => candidate.id === selectedWorkflowId)
    ) {
      setSelectedWorkflowId(workflows[0].id);
    }
  }, [selectedWorkflowId, workflows]);

  const selectedNode = useMemo(
    () => workflow?.nodes.find((node) => node.selected === true) ?? null,
    [workflow],
  );
  const inspectorAvailable = selectedNode !== null || running || runResult !== null;

  useEffect(() => {
    if (inspectorAvailable) {
      expandInspector();
    } else {
      animateInspectorTo(0);
    }
  }, [inspectorAvailable]);

  useEffect(
    () => () => {
      cancelWorkflowPanelAnimation(libraryAnimationRef);
      cancelWorkflowPanelAnimation(inspectorAnimationRef);
    },
    [],
  );

  /** Collapses the workflow library while keeping its last expanded width available. */
  function collapseLibrary(): void {
    animateLibraryTo(0);
  }

  /** Restores the workflow library to the last width chosen by the user. */
  function expandLibrary(): void {
    if (
      inspectorAvailable
      && (editorLayoutRef.current?.getBoundingClientRect().width ?? Number.POSITIVE_INFINITY)
        < NARROW_WORKFLOW_EDITOR_WIDTH
    ) {
      // Sequencing the swap preserves canvas space and gives the inspector room
      // to finish its exit instead of letting both panels fight the constraints.
      animateInspectorTo(0, () => {
        setLibraryCollapsed(false);
        animateLibraryTo(libraryWidthRef.current);
      });
      return;
    }
    setLibraryCollapsed(false);
    animateLibraryTo(libraryWidthRef.current);
  }

  /** Opens the contextual inspector and yields library space first on narrow editors. */
  function expandInspector(): void {
    if (
      (editorLayoutRef.current?.getBoundingClientRect().width ?? Number.POSITIVE_INFINITY)
      < NARROW_WORKFLOW_EDITOR_WIDTH
    ) {
      animateLibraryTo(0, () => {
        setInspectorCollapsed(false);
        animateInspectorTo(inspectorWidthRef.current);
      });
      return;
    }
    setInspectorCollapsed(false);
    animateInspectorTo(inspectorWidthRef.current);
  }

  /** Clears node context and collapses the inspector without affecting workflow edits. */
  function closeNodeInspector(): void {
    updateWorkflow((current) => ({
      ...current,
      nodes: current.nodes.map((node) => ({ ...node, selected: false })),
    }));
    animateInspectorTo(0);
  }

  /** Moves the library to a stable width with the shared panel motion behavior. */
  function animateLibraryTo(
    targetWidth: number,
    onComplete?: () => void,
  ): void {
    animateWorkflowPanel({
      animationRef: libraryAnimationRef,
      duration: WORKFLOW_PANEL_SETTLE_DURATION,
      onCollapsed: () => setLibraryCollapsed(true),
      onComplete,
      panel: libraryPanelRef.current,
      targetWidth,
    });
  }

  /** Moves the inspector to a stable width with the shared panel motion behavior. */
  function animateInspectorTo(
    targetWidth: number,
    onComplete?: () => void,
  ): void {
    animateWorkflowPanel({
      animationRef: inspectorAnimationRef,
      duration: WORKFLOW_PANEL_SETTLE_DURATION,
      onCollapsed: () => setInspectorCollapsed(true),
      onComplete,
      panel: inspectorPanelRef.current,
      targetWidth,
    });
  }

  /** Snaps an undersized library only after release so direct dragging stays linear. */
  function settleLibraryAfterUserResize(): void {
    const width = libraryCurrentWidthRef.current;
    if (width <= 0 || width >= MIN_WORKFLOW_LIBRARY_WIDTH) {
      return;
    }
    animateLibraryTo(
      width < WORKFLOW_LIBRARY_COLLAPSE_THRESHOLD
        ? 0
        : MIN_WORKFLOW_LIBRARY_WIDTH,
    );
  }

  /** Snaps an undersized inspector only after release, never while it tracks the pointer. */
  function settleInspectorAfterUserResize(): void {
    const width = inspectorCurrentWidthRef.current;
    if (width <= 0 || width >= MIN_WORKFLOW_INSPECTOR_WIDTH) {
      return;
    }
    animateInspectorTo(
      width < WORKFLOW_INSPECTOR_COLLAPSE_THRESHOLD
        ? 0
        : MIN_WORKFLOW_INSPECTOR_WIDTH,
    );
  }

  /** Applies one graph or metadata mutation to the current in-memory demo. */
  function updateWorkflow(
    updater: (current: DemoWorkflow) => DemoWorkflow,
  ): void {
    setWorkflows((current) =>
      current.map((candidate) =>
        candidate.id === selectedWorkflowId ? updater(candidate) : candidate,
      ),
    );
  }

  /** Switches the active graph while preserving its React Flow snapshot. */
  function selectWorkflow(workflowId: string): void {
    runRequestRef.current += 1;
    setRunning(false);
    setSelectedWorkflowId(workflowId);
    setRunResult(null);
    setManagerError(null);
  }

  /** Creates a usable blank workflow and immediately opens it for editing. */
  function createWorkflow(name: string): void {
    setManagerError(null);
    const { id } = uniqueGraphId("workflow", workflows.map((item) => item.id));
    const created = createDemoWorkflow(id, name, locale);
    setWorkflows((current) => [...current, created]);
    selectWorkflow(created.id);
  }

  /** Renames one workflow for the lifetime of the current demo session. */
  function renameWorkflow(workflowId: string, name: string): void {
    const nextName = name.trim();
    if (nextName === "") {
      return;
    }
    setWorkflows((current) =>
      current.map((candidate) =>
        candidate.id === workflowId ? { ...candidate, name: nextName } : candidate,
      ),
    );
  }

  /** Deletes a workflow and selects the nearest remaining item to avoid a dead editor state. */
  function deleteWorkflow(workflowId: string): void {
    setManagerError(null);
    setWorkflows((current) => current.filter((candidate) => candidate.id !== workflowId));
    if (selectedWorkflowId === workflowId) {
      runRequestRef.current += 1;
      setSelectedWorkflowId(null);
      setRunResult(null);
    }
  }

  /** Parses and validates an exported workflow before adding it to session state. */
  async function importWorkflow(file: File): Promise<void> {
    setManagerError(null);
    try {
      const imported = parseDemoWorkflow(JSON.parse(await file.text()));
      const ids = new Set(workflows.map((candidate) => candidate.id));
      if (ids.has(imported.id)) {
        imported.id = uniqueGraphId(`${imported.id}-imported`, ids).id;
      }
      setWorkflows((current) => [...current, imported]);
      selectWorkflow(imported.id);
    } catch {
      setManagerError(t("settings.workflow.importError"));
    }
  }

  /** Adds a catalog node at a canvas-provided position and selects it for immediate editing. */
  function addNode(kind: WorkflowNodeKind, position: XYPosition): void {
    if (
      workflow === null
      || (kind === "start" && workflow.nodes.some((node) => node.data.kind === "start"))
    ) {
      return;
    }
    const { sequence } = uniqueGraphId(kind, [
      ...workflow.nodes.map((node) => node.id),
      ...workflow.edges.map((edge) => edge.id),
    ]);
    const node = createMockWorkflowNode({
      kind,
      sequence,
      position,
      locale,
    });
    updateWorkflow((current) => ({
      ...current,
      nodes: [
        ...current.nodes.map((candidate) => ({ ...candidate, selected: false })),
        { ...node, selected: true },
      ],
    }));
    expandInspector();
  }

  /** Creates a native React Flow edge after canvas validation succeeds. */
  function connectNodes(connection: Connection): void {
    updateWorkflow((current) => {
      if (connection.source === null || connection.target === null) {
        return current;
      }
      const { id } = uniqueGraphId("edge", [
        ...current.nodes.map((node) => node.id),
        ...current.edges.map((edge) => edge.id),
      ]);
      return {
        ...current,
        edges: addEdge({ ...connection, id, type: "workflow" }, current.edges),
      };
    });
  }

  /** Uses React Flow's reconnect helper to move an edge endpoint. */
  function reconnectEdge(edge: Edge, connection: Connection): void {
    updateWorkflow((current) => ({
      ...current,
      edges: reconnectReactFlowEdge(edge, connection, current.edges),
    }));
  }

  /** Applies React Flow node changes directly to the active graph. */
  function changeNodes(changes: NodeChange<Node<WorkflowNodeData, "workflow">>[]): void {
    updateWorkflow((current) => ({
      ...current,
      nodes: applyNodeChanges<Node<WorkflowNodeData, "workflow">>(changes, current.nodes),
    }));
  }

  /** Applies React Flow edge changes directly to the active graph. */
  function changeEdges(changes: EdgeChange[]): void {
    updateWorkflow((current) => ({
      ...current,
      edges: applyEdgeChanges(changes, current.edges),
    }));
  }

  /** Stores React Flow's viewport alongside its nodes and edges for exact restoration. */
  function changeViewport(viewport: Viewport): void {
    updateWorkflow((current) => ({ ...current, viewport }));
  }

  /** Runs the deterministic mock preview and exposes progress before showing its trace. */
  async function runWorkflow(input: string): Promise<void> {
    if (workflow === null) {
      return;
    }
    expandInspector();
    const request = ++runRequestRef.current;
    const draft = workflow;
    setRunning(true);
    setRunResult(null);
    try {
      const result = await runDemoWorkflow(draft, input, locale);
      if (runRequestRef.current === request) {
        setRunResult(result);
      }
    } catch {
      if (runRequestRef.current === request) {
        setRunResult({
          status: "failed",
          durationMs: 0,
          output: t("settings.workflow.runError"),
          steps: [],
        });
      }
    } finally {
      if (runRequestRef.current === request) {
        setRunning(false);
      }
    }
  }

  return (
    <div
      className="flex h-full min-h-0 flex-col bg-background"
      onKeyDown={(event) => {
        if (
          event.key === "Escape"
          && !event.defaultPrevented
          && selectedNode !== null
          && !running
          && runResult === null
        ) {
          event.preventDefault();
          event.stopPropagation();
          closeNodeInspector();
        }
      }}
    >
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
                  DEMO
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
      </header>
      <div ref={editorLayoutRef} className="min-h-0 flex-1">
        <ResizablePanelGroup
          orientation="horizontal"
          resizeTargetMinimumSize={{ coarse: 28, fine: 12 }}
          onLayoutChanged={(_layout, meta) => {
            if (meta.isUserInteraction) {
              settleLibraryAfterUserResize();
              settleInspectorAfterUserResize();
            }
          }}
        >
          <ResizablePanel
            id="workflow-library"
            panelRef={libraryPanelRef}
            defaultSize={initialLibraryWidth}
            minSize={1}
            maxSize={MAX_WORKFLOW_LIBRARY_WIDTH}
            collapsedSize={0}
            collapsible
            groupResizeBehavior="preserve-pixel-size"
            onResize={(size) => {
              const collapsed = size.inPixels < 1;
              libraryCurrentWidthRef.current = size.inPixels;
              setLibraryVisualWidth(size.inPixels);
              setLibraryCollapsed(collapsed);
              if (size.inPixels >= MIN_WORKFLOW_LIBRARY_WIDTH) {
                libraryWidthRef.current = size.inPixels;
              }
            }}
          >
            <div
              aria-hidden={libraryCollapsed}
              className="flex min-h-0 flex-1"
              style={{
                opacity: Math.max(
                  0,
                  Math.min(
                    1,
                    (libraryVisualWidth - WORKFLOW_LIBRARY_FADE_START)
                      / (MIN_WORKFLOW_LIBRARY_WIDTH - WORKFLOW_LIBRARY_FADE_START),
                  ),
                ),
              }}
            >
              <WorkflowManager
                workflows={workflows}
                selectedWorkflowId={selectedWorkflowId}
                error={managerError}
                onSelect={selectWorkflow}
                onCreate={createWorkflow}
                onRename={renameWorkflow}
                onDelete={deleteWorkflow}
                onImport={(file) => void importWorkflow(file)}
                onCollapse={collapseLibrary}
              />
            </div>
          </ResizablePanel>
          <ResizableHandle
            withHandle
            aria-label={t("settings.workflow.resizeLibrary")}
            title={t("settings.workflow.resizeLibrary")}
            className="z-20 after:w-3 transition-colors hover:bg-ring focus-visible:bg-ring"
            onPointerDown={() => cancelWorkflowPanelAnimation(libraryAnimationRef)}
            onDoubleClick={() => {
              libraryWidthRef.current = DEFAULT_WORKFLOW_LIBRARY_WIDTH;
              libraryPanelRef.current?.resize(DEFAULT_WORKFLOW_LIBRARY_WIDTH);
            }}
          />
          <ResizablePanel id="workflow-canvas" minSize={MIN_WORKFLOW_CANVAS_WIDTH}>
            {workflow === null ? (
              <WorkflowEmpty
                onCreate={() =>
                  createWorkflow(
                    t("settings.workflow.untitledWorkflow", { count: workflows.length + 1 }),
                  )
                }
              />
            ) : (
              <WorkflowCanvas
                key={workflow.id}
                capabilities={capabilities}
                nodes={workflow.nodes}
                edges={workflow.edges}
                initialViewport={workflow.viewport}
                onNodesChange={changeNodes}
                onEdgesChange={changeEdges}
                onViewportChange={changeViewport}
                onAddNode={addNode}
                onConnect={connectNodes}
                onReconnect={reconnectEdge}
                libraryCollapsed={libraryCollapsed}
                inspectorCollapsed={inspectorCollapsed}
                inspectorAvailable={inspectorAvailable}
                onExpandLibrary={expandLibrary}
                onExpandInspector={expandInspector}
              />
            )}
          </ResizablePanel>
          <ResizableHandle
            withHandle
            aria-label={t("settings.workflow.resizeConfiguration")}
            title={t("settings.workflow.resizeConfiguration")}
            className="z-20 after:w-3 transition-colors hover:bg-ring focus-visible:bg-ring"
            onPointerDown={() => cancelWorkflowPanelAnimation(inspectorAnimationRef)}
            onDoubleClick={() => {
              inspectorWidthRef.current = DEFAULT_WORKFLOW_INSPECTOR_WIDTH;
              inspectorPanelRef.current?.resize(DEFAULT_WORKFLOW_INSPECTOR_WIDTH);
            }}
          />
          <ResizablePanel
            id="workflow-inspector"
            panelRef={inspectorPanelRef}
            defaultSize={0}
            minSize={1}
            maxSize={MAX_WORKFLOW_INSPECTOR_WIDTH}
            collapsedSize={0}
            collapsible
            groupResizeBehavior="preserve-pixel-size"
            onResize={(size) => {
              const collapsed = size.inPixels < 1;
              inspectorCurrentWidthRef.current = size.inPixels;
              setInspectorVisualWidth(size.inPixels);
              setInspectorCollapsed(collapsed);
              if (size.inPixels >= MIN_WORKFLOW_INSPECTOR_WIDTH) {
                inspectorWidthRef.current = size.inPixels;
              }
            }}
          >
            <div
              aria-hidden={inspectorCollapsed}
              className="flex min-h-0 flex-1"
              style={{
                opacity: Math.max(
                  0,
                  Math.min(
                    1,
                    (inspectorVisualWidth - WORKFLOW_INSPECTOR_FADE_START)
                      / (MIN_WORKFLOW_INSPECTOR_WIDTH - WORKFLOW_INSPECTOR_FADE_START),
                  ),
                ),
              }}
            >
              <WorkflowInspector
                node={selectedNode}
                running={running}
                runResult={runResult}
                capabilities={capabilities}
                onUpdate={(updatedNode) =>
                  updateWorkflow((current) => ({
                    ...current,
                    nodes: current.nodes.map((node) =>
                      node.id === updatedNode.id ? updatedNode : node,
                    ),
                  }))
                }
                onDelete={(nodeId) => {
                  void deleteElements({ nodes: [{ id: nodeId }] });
                }}
                onCloseNode={closeNodeInspector}
                onCloseRun={() => setRunResult(null)}
                onRun={(input) => void runWorkflow(input)}
              />
            </div>
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>
    </div>
  );
}

/** Gives an empty collection a clear recovery action without disguising it as a loading state. */
function WorkflowEmpty({ onCreate }: { onCreate: () => void }) {
  const { t } = useTranslation();
  return (
    <section className="flex min-h-0 flex-1 items-center justify-center bg-muted/25">
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
