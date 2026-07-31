import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconCheck,
  IconCloudCheck,
  IconPlayerPlay,
  IconRoute,
} from "@tabler/icons-react";
import {
  Button,
  Input,
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  Skeleton,
  type ResizablePanelHandle,
} from "@ora/ui";
import {
  createMockWorkflowCapabilities,
  createMockWorkflowNode,
  MockWorkflowRepository,
  type WorkflowCapabilities,
  type WorkflowDefinition,
  type WorkflowLocale,
  type WorkflowNodeKind,
  type WorkflowPosition,
  type WorkflowRepository,
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
const WORKFLOW_LIBRARY_WIDTH_KEY = "ora.workflow.library-width";
const WORKFLOW_INSPECTOR_WIDTH_KEY = "ora.workflow.inspector-width";

export interface WorkflowSettingsProps {
  repository?: WorkflowRepository;
  capabilities?: WorkflowCapabilities;
}

/** Finds the first readable graph ID that does not collide with imported or persisted elements. */
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

/** Restores a valid panel width without letting stale storage break the editor layout. */
function storedPanelWidth(
  key: string,
  fallback: number,
  minimum: number,
  maximum: number,
): number {
  try {
    const stored = Number.parseFloat(window.localStorage.getItem(key) ?? "");
    return Number.isFinite(stored)
      ? Math.min(maximum, Math.max(minimum, stored))
      : fallback;
  } catch {
    return fallback;
  }
}

/** Persists only expanded panel sizes so collapsing never overwrites the user's preference. */
function rememberPanelWidth(key: string, width: number): void {
  try {
    window.localStorage.setItem(key, String(Math.round(width)));
  } catch {
    // Storage can be unavailable in restricted webviews; resizing should still work in memory.
  }
}

/** Owns the frontend-only workflow editor state and coordinates the mock repository boundary. */
export function WorkflowSettings({
  repository: repositoryOverride,
  capabilities: capabilitiesOverride,
}: WorkflowSettingsProps = {}) {
  const { i18n, t } = useTranslation();
  const locale: WorkflowLocale = i18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN";
  const [defaultRepository] = useState(() => new MockWorkflowRepository(locale));
  const repository = repositoryOverride ?? defaultRepository;
  const capabilities = useMemo(
    () => capabilitiesOverride ?? createMockWorkflowCapabilities(locale),
    [capabilitiesOverride, locale],
  );
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
  const workflowRevisionsRef = useRef(new Map<string, number>());
  const runRequestRef = useRef(0);
  const editorLayoutRef = useRef<HTMLDivElement>(null);
  const libraryPanelRef = useRef<ResizablePanelHandle | null>(null);
  const inspectorPanelRef = useRef<ResizablePanelHandle | null>(null);
  const libraryAnimationRef = useRef<number | null>(null);
  const inspectorAnimationRef = useRef<number | null>(null);
  const [initialLibraryWidth] = useState(() =>
    storedPanelWidth(
      WORKFLOW_LIBRARY_WIDTH_KEY,
      DEFAULT_WORKFLOW_LIBRARY_WIDTH,
      MIN_WORKFLOW_LIBRARY_WIDTH,
      MAX_WORKFLOW_LIBRARY_WIDTH,
    ),
  );
  const [initialInspectorWidth] = useState(() =>
    storedPanelWidth(
      WORKFLOW_INSPECTOR_WIDTH_KEY,
      DEFAULT_WORKFLOW_INSPECTOR_WIDTH,
      MIN_WORKFLOW_INSPECTOR_WIDTH,
      MAX_WORKFLOW_INSPECTOR_WIDTH,
    ),
  );
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

  useEffect(() => {
    if (
      !loading
      && workflows.length > 0
      && !workflows.some((candidate) => candidate.id === selectedWorkflowId)
    ) {
      setSelectedWorkflowId(workflows[0].id);
    }
  }, [loading, selectedWorkflowId, workflows]);

  const selectedNode = useMemo(
    () => workflow?.nodes.find((node) => node.id === selectedNodeId) ?? null,
    [selectedNodeId, workflow],
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
    setSelectedNodeId(null);
    animateInspectorTo(0);
  }

  /** Synchronizes node selection with the contextual inspector's visibility. */
  function selectNode(nodeId: string | null): void {
    setSelectedNodeId(nodeId);
    if (nodeId === null) {
      if (!running && runResult === null) {
        animateInspectorTo(0);
      }
      return;
    }
    expandInspector();
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
      workflowRevisionsRef.current.set(
        selectedWorkflowId,
        (workflowRevisionsRef.current.get(selectedWorkflowId) ?? 0) + 1,
      );
      setDirtyWorkflowIds((current) => new Set(current).add(selectedWorkflowId));
    }
  }

  /** Switches the active graph and clears transient state that belongs to the previous workflow. */
  function selectWorkflow(workflowId: string): void {
    runRequestRef.current += 1;
    setRunning(false);
    setSelectedWorkflowId(workflowId);
    setSelectedNodeId(null);
    setRunResult(null);
    setManagerError(null);
  }

  /** Creates a usable blank workflow and immediately opens it for editing. */
  async function createWorkflow(name: string): Promise<void> {
    setManaging(true);
    setManagerError(null);
    try {
      const created = await repository.create(name);
      setWorkflows((current) => [...current, created]);
      selectWorkflow(created.id);
    } catch {
      setManagerError(t("settings.workflow.manageError"));
    } finally {
      setManaging(false);
    }
  }

  /** Renames one workflow in local state and marks it dirty for explicit save. */
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
    workflowRevisionsRef.current.set(
      workflowId,
      (workflowRevisionsRef.current.get(workflowId) ?? 0) + 1,
    );
    setDirtyWorkflowIds((current) => new Set(current).add(workflowId));
  }

  /** Deletes a workflow and selects the nearest remaining item to avoid a dead editor state. */
  async function deleteWorkflow(workflowId: string): Promise<void> {
    setManaging(true);
    setManagerError(null);
    try {
      await repository.delete(workflowId);
      setWorkflows((current) =>
        current.filter((candidate) => candidate.id !== workflowId),
      );
      setSelectedWorkflowId((current) => {
        if (current === workflowId) {
          runRequestRef.current += 1;
          return null;
        }
        return current;
      });
      if (selectedWorkflowId === workflowId) {
        setSelectedNodeId(null);
        setRunResult(null);
      }
      workflowRevisionsRef.current.delete(workflowId);
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
  function addNode(kind: WorkflowNodeKind, position: WorkflowPosition): void {
    if (workflow === null) {
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
    updateWorkflow((current) => ({ ...current, nodes: [...current.nodes, node] }));
    setSelectedNodeId(node.id);
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
        source === target
        || !current.nodes.some((node) => node.id === source)
        || !current.nodes.some((node) => node.id === target)
        || current.edges.some((edge) => edge.source === source && edge.target === target)
      ) {
        return current;
      }
      const { id } = uniqueGraphId("edge", [
        ...current.nodes.map((node) => node.id),
        ...current.edges.map((edge) => edge.id),
      ]);
      return {
        ...current,
        edges: [
          ...current.edges,
          {
            id,
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
    const savingRevision = workflowRevisionsRef.current.get(workflow.id) ?? 0;
    try {
      const persisted = await repository.save(workflow);
      if ((workflowRevisionsRef.current.get(persisted.id) ?? 0) === savingRevision) {
        setWorkflows((current) =>
          current.map((candidate) => candidate.id === persisted.id ? persisted : candidate),
        );
        setDirtyWorkflowIds((current) => {
          const next = new Set(current);
          next.delete(persisted.id);
          return next;
        });
      }
    } catch {
      setManagerError(t("settings.workflow.manageError"));
    } finally {
      setSaving(false);
    }
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
      const result = await repository.run(draft, input);
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

  if (loading) {
    return <WorkflowLoading />;
  }

  return (
    <div
      className="flex h-full min-h-0 flex-col bg-background"
      onKeyDown={(event) => {
        if (
          event.key === "Escape"
          && !event.defaultPrevented
          && selectedNodeId !== null
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
                {repository.dataSourceKind === "mock" && (
                  <span className="hidden rounded-full border border-border px-2 py-0.5 text-[9px] font-medium text-muted-foreground sm:inline">
                    MOCK
                  </span>
                )}
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
                rememberPanelWidth(WORKFLOW_LIBRARY_WIDTH_KEY, size.inPixels);
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
                busy={managing}
                error={managerError}
                onSelect={selectWorkflow}
                onCreate={(name) => void createWorkflow(name)}
                onRename={renameWorkflow}
                onDelete={(workflowId) => void deleteWorkflow(workflowId)}
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
              rememberPanelWidth(
                WORKFLOW_LIBRARY_WIDTH_KEY,
                DEFAULT_WORKFLOW_LIBRARY_WIDTH,
              );
              libraryPanelRef.current?.resize(DEFAULT_WORKFLOW_LIBRARY_WIDTH);
            }}
          />
          <ResizablePanel id="workflow-canvas" minSize={MIN_WORKFLOW_CANVAS_WIDTH}>
            {workflow === null ? (
              <WorkflowEmpty
                onCreate={() =>
                  void createWorkflow(
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
                selectedNodeId={selectedNodeId}
                onSelectNode={selectNode}
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
              rememberPanelWidth(
                WORKFLOW_INSPECTOR_WIDTH_KEY,
                DEFAULT_WORKFLOW_INSPECTOR_WIDTH,
              );
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
                rememberPanelWidth(WORKFLOW_INSPECTOR_WIDTH_KEY, size.inPixels);
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
                onDelete={deleteNode}
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
