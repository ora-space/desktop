import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
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
  type ReactFlowJsonObject,
  type XYPosition,
} from "@xyflow/react";
import {
  IconArrowLeft,
  IconDownload,
  IconLayoutSidebarLeftExpand,
  IconRoute,
  IconVersions,
} from "@tabler/icons-react";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Input,
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  toast,
  type ResizablePanelHandle,
} from "@ora/ui";
import { DragRegion } from "../../components/drag-region";
import { WindowControls } from "../../components/window-controls";
import { useUiStore } from "../../state/stores/ui-store";
import {
  createMockWorkflowCapabilities,
  createMockWorkflowNode,
  normalizeWorkflowNodeAgentConfigs,
  type DemoWorkflow,
  type MockWorkflowVersion,
  type WorkflowCapabilities,
  type WorkflowAnnotationData,
  type WorkflowAnnotationNode,
  type WorkflowNodeData,
  type WorkflowNodeKind,
} from "@ora/workflow-mock";
import {
  normalizeWorkflowDefinition,
  parseWorkflowGraph,
  serializeWorkflowGraph,
  workflowTimestampToIso,
} from "@ora/workflow-runtime";
import { usePlatform } from "../../platform";
import { useContractsClient } from "../../contracts-client-context";
import { useAgents } from "../../state/hooks/use-agents";
import { availableSkills, useSkills } from "../../state/hooks/use-skills";
import { useWorkflowAgentModels } from "../../state/hooks/use-workflow-agent-models";
import { localizeContractError } from "../../i18n/contract-error";
import { WorkflowCanvas } from "./workflow-canvas";
import { organizeWorkflowNodes } from "./workflow-flow/layout";
import type { WorkflowCanvasNode } from "./workflow-flow/types";
import { WorkflowInspector } from "./workflow-inspector";
import { MCP_CATALOG } from "./mcp-catalog";
import {
  useWorkflowEditorStore,
  type WorkflowEditorLibraryActions,
} from "./workflow-editor-store";
import {
  useActivateWorkflow,
  useCopyWorkflow,
  useCreateWorkflow,
  useDeleteWorkflow,
  useDeleteWorkflowSnapshot,
  usePublishWorkflow,
  useRenameWorkflow,
  useUpdateWorkflowDraft,
  useWorkflowDraft,
  useWorkflowLibrary,
  useWorkflowVersions,
} from "./workflow-definitions";
import { WorkflowDraftSaveStatusLabel } from "./workflow-draft-save-status";
import { useWorkflowDraftAutosave } from "./use-workflow-draft-autosave";
import {
  animatePanelWidth as animateWorkflowPanel,
  cancelPanelWidthAnimation as cancelWorkflowPanelAnimation,
} from "../../lib/panel-motion";

const DEFAULT_WORKFLOW_INSPECTOR_WIDTH = 320;
const MIN_WORKFLOW_INSPECTOR_WIDTH = 240;
const MAX_WORKFLOW_INSPECTOR_WIDTH = 480;
const WORKFLOW_INSPECTOR_COLLAPSE_THRESHOLD = 180;
const WORKFLOW_INSPECTOR_FADE_START = 120;
const WORKFLOW_PANEL_SETTLE_DURATION = 180;
const MIN_WORKFLOW_CANVAS_WIDTH = 360;

export interface WorkflowEditorProps {
  capabilities?: WorkflowCapabilities;
}

/** Finds the first readable graph ID that does not collide with session elements. */
function uniqueGraphId(
  prefix: string,
  existingIds: Iterable<string>,
): {
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

/**
 * Keeps a still-valid selection, otherwise the first remaining library row.
 * Derived during render so a cached library can start the draft query on the
 * first paint instead of flashing the loading pane until an effect writes the id.
 *
 * Create prepends the new summary to the library cache before writing this id,
 * so a just-created workflow is already "in the library" here. A missing id is
 * treated as deleted so the first remaining row can take over.
 */
function resolveSelectedWorkflowId(
  selectedWorkflowId: string | null,
  library: readonly { id: string }[] | undefined,
): string | null {
  if (library === undefined) {
    return selectedWorkflowId;
  }
  if (
    selectedWorkflowId !== null &&
    library.some((item) => item.id === selectedWorkflowId)
  ) {
    return selectedWorkflowId;
  }
  return library[0]?.id ?? null;
}

/** Narrows editor notes so they can be split from executable React Flow nodes. */
function isWorkflowAnnotationNode(
  node: WorkflowCanvasNode,
): node is WorkflowAnnotationNode {
  return node.type === "annotation";
}

/** Rebuilds the persisted workflow from React Flow's mixed editor-node snapshot. */
function workflowFromCanvasSnapshot(
  workflow: DemoWorkflow,
  graph: ReactFlowJsonObject<WorkflowCanvasNode, Edge>,
): DemoWorkflow {
  const annotationDataById = new Map(
    (workflow.annotations ?? []).map((annotation) => [
      annotation.id,
      annotation.data,
    ]),
  );
  return {
    ...workflow,
    nodes: graph.nodes.filter(
      (node): node is Node<WorkflowNodeData, "workflow"> =>
        !isWorkflowAnnotationNode(node),
    ),
    annotations: graph.nodes
      .filter(isWorkflowAnnotationNode)
      .map((annotation) => ({
        ...annotation,
        // React Flow owns live geometry, while React state owns note content and
        // may be one render ahead when an autosave timer fires after typing.
        data: annotationDataById.get(annotation.id) ?? annotation.data,
      })),
    edges: graph.edges,
    viewport: graph.viewport,
  };
}

/** Produces a portable filename while retaining the workflow name for the save dialog. */
function workflowExportFileName(name: string): string {
  // `\p{Cc}` is the Unicode Control category; property escapes keep control
  // characters out of the regex literal so no-control-regex stays satisfied.
  const safeName = name.replace(/[<>:"/\\|?*\p{Cc}]/gu, " ").trim();
  return `${safeName === "" ? "workflow" : safeName}.reactflow.json`;
}

/**
 * Picks a publish version for an imported file: prefer the filename stem (matching export
 * naming), then the workflow title, else let the backend mint an automatic version.
 */
function importPublishVersion(
  fileName: string,
  workflowName: string,
): string | null {
  const stem = fileName
    .replace(/\.reactflow\.json$/i, "")
    .replace(/\.json$/i, "")
    .trim();
  const candidate = (stem !== "" ? stem : workflowName).trim();
  if (
    candidate === "" ||
    candidate === "draft" ||
    candidate === "." ||
    candidate === ".." ||
    candidate.length > 128 ||
    [...candidate].some(
      (character) =>
        character === "/" || character === "\\" || character.charCodeAt(0) < 32,
    )
  ) {
    return null;
  }
  return candidate;
}

/** Provides one React Flow store to the canvas and its sibling inspector. */
export function WorkflowEditor(props: WorkflowEditorProps = {}) {
  return (
    <ReactFlowProvider>
      <WorkflowEditorContent {...props} />
    </ReactFlowProvider>
  );
}

/** Owns the selected draft graph and publishes library actions to the sidebar list. */
function WorkflowEditorContent({
  capabilities: capabilitiesOverride,
}: WorkflowEditorProps) {
  const { i18n, t } = useTranslation();
  const platform = usePlatform();
  const client = useContractsClient();
  const agentsQuery = useAgents();
  const skillsQuery = useSkills();
  const agentModelsCatalog = useWorkflowAgentModels();
  const { deleteElements, toObject } = useReactFlow<WorkflowCanvasNode, Edge>();
  const locale =
    i18n.resolvedLanguage === "en-US" ? ("en-US" as const) : ("zh-CN" as const);
  /**
   * Uses backend-managed Agent/Skill catalogs and warm-session model discovery
   * for node configuration while preserving demo-only tool catalogs.
   */
  const capabilities = useMemo(() => {
    if (capabilitiesOverride !== undefined) {
      return capabilitiesOverride;
    }
    const baseCapabilities = createMockWorkflowCapabilities(locale);
    // Store the agent/skill name in the JSON (roleId / skillId), so exported workflows are
    // readable and portable across Ora instances instead of opaque catalog ids.
    const roles = (agentsQuery.data ?? []).map((agent) => ({
      value: agent.name,
      label: agent.name,
    }));
    const skills = availableSkills(skillsQuery.data ?? []).map((skill) => ({
      value: skill.name,
      label: skill.name,
    }));
    const mcps = MCP_CATALOG.map((mcp) => ({
      value: mcp.id,
      label: mcp.name,
    }));
    const agentModels = agentModelsCatalog.agentModels;
    const defaultExecutor = agentModels[0];
    return {
      ...baseCapabilities,
      agentModels,
      roles,
      skills,
      mcps,
      defaultAgentConfig: {
        ...baseCapabilities.defaultAgentConfig,
        ...(defaultExecutor === undefined
          ? {}
          : {
              executor: {
                agentCli: defaultExecutor.agentCli,
                modelId: defaultExecutor.modelId,
              },
            }),
        // Roles are optional; a new agent node starts with no role selected.
        roleId: "",
        mcps: [],
      },
    };
  }, [
    agentModelsCatalog.agentModels,
    agentsQuery.data,
    capabilitiesOverride,
    locale,
    skillsQuery.data,
  ]);
  const agentModelsLoading =
    capabilitiesOverride === undefined && agentModelsCatalog.isLoading;
  const agentModelsError =
    capabilitiesOverride === undefined && agentModelsCatalog.isError;
  const availableAgentModels = agentModelsCatalog.agentModels;
  const agentCatalogsLoading = agentsQuery.isPending || skillsQuery.isPending;
  const agentCatalogsError =
    agentsQuery.error !== null || skillsQuery.error !== null;

  const library = useWorkflowLibrary();
  const selectedWorkflowId = useWorkflowEditorStore(
    (state) => state.selectedWorkflowId,
  );
  const setSelectedWorkflowId = useWorkflowEditorStore(
    (state) => state.setSelectedWorkflowId,
  );
  const managerError = useWorkflowEditorStore((state) => state.managerError);
  const setManagerError = useWorkflowEditorStore(
    (state) => state.setManagerError,
  );
  const registerActions = useWorkflowEditorStore(
    (state) => state.registerActions,
  );
  const sidebarCollapsed = useUiStore((state) => state.sidebarCollapsed);
  const setSidebarCollapsed = useUiStore((state) => state.setSidebarCollapsed);
  const setWorkflowEditorOpen = useUiStore(
    (state) => state.setWorkflowEditorOpen,
  );
  const resolvedWorkflowId = resolveSelectedWorkflowId(
    selectedWorkflowId,
    library.data,
  );
  const draftQuery = useWorkflowDraft(resolvedWorkflowId);
  const versionsQuery = useWorkflowVersions(resolvedWorkflowId);
  const createWorkflowMutation = useCreateWorkflow();
  const copyWorkflowMutation = useCopyWorkflow();
  const renameWorkflowMutation = useRenameWorkflow();
  const deleteWorkflowMutation = useDeleteWorkflow();
  const updateDraftMutation = useUpdateWorkflowDraft();
  const publishWorkflowMutation = usePublishWorkflow();
  const activateWorkflowMutation = useActivateWorkflow();
  const deleteSnapshotMutation = useDeleteWorkflowSnapshot();

  const [workflow, setWorkflow] = useState<DemoWorkflow | null>(null);
  /** Selected workflow id whose draft is currently mounted in the editor. */
  const [hydratedWorkflowId, setHydratedWorkflowId] = useState<string | null>(
    null,
  );
  const [previewedVersion, setPreviewedVersion] =
    useState<MockWorkflowVersion | null>(null);
  const [publishDialogOpen, setPublishDialogOpen] = useState(false);
  const [publishVersionName, setPublishVersionName] = useState("");
  const inspectorPanelRef = useRef<ResizablePanelHandle | null>(null);
  const inspectorAnimationRef = useRef<number | null>(null);
  /** Bumps on every persistable edit so in-flight writes can detect they are stale. */
  const editGenerationRef = useRef(0);
  const workflowRef = useRef<DemoWorkflow | null>(null);
  const previewedVersionRef = useRef<MockWorkflowVersion | null>(null);
  /** Last name known to be persisted, so autosave skips no-op renames. */
  const persistedNameRef = useRef<string | null>(null);
  const libraryActionsRef = useRef<WorkflowEditorLibraryActions | null>(null);
  const initialInspectorWidth = DEFAULT_WORKFLOW_INSPECTOR_WIDTH;
  const inspectorWidthRef = useRef(initialInspectorWidth);
  const inspectorCurrentWidthRef = useRef(0);
  const [inspectorCollapsed, setInspectorCollapsed] = useState(true);
  const [inspectorVisualWidth, setInspectorVisualWidth] = useState(0);

  // Autosave flush reads these after render; keep them current without render-time writes.
  useEffect(() => {
    workflowRef.current = workflow;
    previewedVersionRef.current = previewedVersion;
  });

  // Render-phase adjustments (the documented "adjust state when props change"
  // pattern): hydrate when the selected workflow identity changes. Autosave must
  // not remount the canvas, so timestamp-only draft updates are ignored here;
  // activate clears hydratedWorkflowId to force a same-id reload.
  if (
    draftQuery.data !== undefined &&
    draftQuery.data.workflow.id === resolvedWorkflowId &&
    hydratedWorkflowId !== resolvedWorkflowId
  ) {
    const envelope = parseWorkflowGraph(draftQuery.data.draft.graph);
    // Persisted drafts may reference a model that is no longer available. Keep the
    // selected CLI stable and only substitute a discovered model for that same CLI.
    // Agent nodes without a contract (legacy prompt/model graphs folded into Agent
    // on parse) get the default executor so the inspector stays editable.
    const nodes = normalizeWorkflowNodeAgentConfigs(
      envelope.nodes
        .map((node) => {
          if (
            node.data.kind !== "agent" ||
            node.data.agentConfig !== undefined
          ) {
            return node;
          }
          return {
            ...node,
            data: {
              ...node.data,
              agentConfig: capabilities.defaultAgentConfig,
            },
          };
        })
        .map((node) => {
          if (
            node.data.kind !== "agent" ||
            node.data.agentConfig === undefined
          ) {
            return node;
          }
          if (
            capabilitiesOverride !== undefined ||
            availableAgentModels.length === 0
          ) {
            return node;
          }
          const { agentCli, modelId } = node.data.agentConfig.executor;
          if (
            availableAgentModels.some(
              (model) =>
                model.agentCli === agentCli && model.modelId === modelId,
            )
          ) {
            return node;
          }
          const modelForCli = availableAgentModels.find(
            (model) => model.agentCli === agentCli,
          );
          return {
            ...node,
            data: {
              ...node.data,
              agentConfig: {
                ...node.data.agentConfig,
                executor: {
                  agentCli,
                  modelId: modelForCli?.modelId ?? modelId,
                },
              },
            },
          };
        }),
    );
    setWorkflow({
      id: draftQuery.data.workflow.id,
      name: draftQuery.data.workflow.name,
      description: envelope.description ?? "",
      updatedAt: workflowTimestampToIso(draftQuery.data.workflow.updatedAt),
      viewport: envelope.viewport,
      nodes,
      edges: envelope.edges,
      annotations: envelope.annotations,
    });
    setHydratedWorkflowId(draftQuery.data.workflow.id);
    // Capture the server name in the same hydrate turn so autosave can skip no-op renames.
    // eslint-disable-next-line react-hooks/refs -- render-phase hydrate pairs this with setState
    persistedNameRef.current = draftQuery.data.workflow.name;
  }

  // Write the derived id to the store before paint so the sidebar highlight
  // matches the draft that the first render already started loading.
  useLayoutEffect(() => {
    if (resolvedWorkflowId !== selectedWorkflowId) {
      setSelectedWorkflowId(resolvedWorkflowId);
    }
  }, [resolvedWorkflowId, selectedWorkflowId, setSelectedWorkflowId]);

  /** Maps persisted version summaries into the editor's version-history shape. */
  const versionHistory: MockWorkflowVersion[] = useMemo(
    () =>
      (versionsQuery.data ?? []).map((version) => ({
        id: version.id,
        version: version.version,
        createdAt: new Intl.DateTimeFormat(i18n.resolvedLanguage, {
          month: "short",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
          second: "2-digit",
        }).format(new Date(Number(version.createdAt))),
        graph: { nodes: [], edges: [], viewport: { x: 0, y: 0, zoom: 1 } },
      })),
    [versionsQuery.data, i18n.resolvedLanguage],
  );

  /** Formatted last-edit time of the draft (workflow_snapshots.updated_at). */
  const draftUpdatedAt = useMemo(() => {
    const updatedAt = draftQuery.data?.draft.updatedAt;
    if (updatedAt == null) {
      return undefined;
    }
    return new Intl.DateTimeFormat(i18n.resolvedLanguage, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }).format(new Date(Number(updatedAt)));
  }, [draftQuery.data?.draft.updatedAt, i18n.resolvedLanguage]);

  const displayedWorkflow = useMemo(
    () =>
      previewedVersion === null || workflow === null
        ? workflow
        : { ...workflow, ...previewedVersion.graph },
    [previewedVersion, workflow],
  );
  const loadCause =
    library.error ?? (resolvedWorkflowId !== null ? draftQuery.error : null);
  const selectedNode = useMemo(
    () =>
      previewedVersion === null
        ? (workflow?.nodes.find((node) => node.selected === true) ?? null)
        : null,
    [previewedVersion, workflow],
  );
  const inspectorAvailable = selectedNode !== null;

  useEffect(
    () => () => {
      cancelWorkflowPanelAnimation(inspectorAnimationRef);
    },
    [],
  );

  /** Moves the inspector to a stable width with the shared panel motion behavior. */
  const animateInspectorTo = useCallback(
    (targetWidth: number, onComplete?: () => void): void => {
      animateWorkflowPanel({
        animationRef: inspectorAnimationRef,
        duration: WORKFLOW_PANEL_SETTLE_DURATION,
        onCollapsed: () => setInspectorCollapsed(true),
        onComplete,
        panel: inspectorPanelRef.current,
        targetWidth,
      });
    },
    [],
  );

  /** Opens the contextual inspector when a node has configuration context. */
  const expandInspector = useCallback((): void => {
    setInspectorCollapsed(false);
    animateInspectorTo(inspectorWidthRef.current);
  }, [animateInspectorTo]);

  // Opening the inspector when a node gains context (and collapsing it when it
  // loses context) is an imperative panel animation, keyed on selection only.
  useEffect(() => {
    if (inspectorAvailable) {
      // Panel width is owned by the resizable host; this is an imperative snap.
      // eslint-disable-next-line react-hooks/set-state-in-effect -- panel animation
      expandInspector();
    } else {
      animateInspectorTo(0);
    }
  }, [animateInspectorTo, expandInspector, inspectorAvailable]);

  /** Clears node context and collapses the inspector without affecting workflow edits. */
  function closeNodeInspector(): void {
    setWorkflow((current) =>
      current === null
        ? current
        : {
            ...current,
            nodes: current.nodes.map((node) => ({ ...node, selected: false })),
          },
    );
    animateInspectorTo(0);
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

  /** Applies one graph or metadata mutation to the open in-memory workflow. */
  function updateWorkflow(
    updater: (current: DemoWorkflow) => DemoWorkflow,
    options: { persist?: boolean } = {},
  ): void {
    setWorkflow((current) => (current === null ? current : updater(current)));
    if (options.persist !== false) {
      editGenerationRef.current += 1;
      autosave.markDirty();
    }
  }

  /** Commits the mounted graph through React Flow's database-ready snapshot boundary. */
  function commitCurrentWorkflowSnapshot(): DemoWorkflow | null {
    if (workflow === null || previewedVersion !== null) {
      return null;
    }
    const snapshot = workflowFromCanvasSnapshot(workflow, toObject());
    setWorkflow(snapshot);
    return snapshot;
  }

  /**
   * Writes the current draft graph and name. Returns whether the write still
   * matches the edit generation that started it, so autosave can reschedule.
   * Reads workflow through a ref so a flush right after setState still sees the
   * latest name/description while toObject() supplies live graph geometry.
   */
  const persistDraft = useCallback(async (): Promise<
    "saved" | "stale" | "skipped" | "failed"
  > => {
    const current = workflowRef.current;
    if (current === null || previewedVersionRef.current !== null) {
      return "skipped";
    }
    const snapshot = workflowFromCanvasSnapshot(current, toObject());
    setWorkflow(snapshot);
    const startedGeneration = editGenerationRef.current;
    setManagerError(null);
    try {
      const definition = normalizeWorkflowDefinition({
        id: snapshot.id,
        name: snapshot.name,
        description: snapshot.description,
        updatedAt: snapshot.updatedAt,
        viewport: snapshot.viewport,
        nodes: snapshot.nodes,
        edges: snapshot.edges,
      });
      await updateDraftMutation.mutateAsync({
        workflowId: snapshot.id,
        graph: serializeWorkflowGraph({
          nodes: definition.nodes,
          edges: definition.edges,
          viewport: definition.viewport,
          annotations: snapshot.annotations ?? [],
          description: definition.description,
        }),
      });
      const nextName = snapshot.name.trim();
      // Skip rename when the title is unchanged so graph-only autosaves stay one request.
      if (nextName !== "" && nextName !== persistedNameRef.current) {
        await renameWorkflowMutation.mutateAsync({
          workflowId: snapshot.id,
          name: nextName,
        });
        persistedNameRef.current = nextName;
      }
      if (editGenerationRef.current !== startedGeneration) {
        return "stale";
      }
      return "saved";
    } catch (cause) {
      setManagerError(localizeContractError(cause, t));
      return "failed";
    }
  }, [
    renameWorkflowMutation,
    setManagerError,
    t,
    toObject,
    updateDraftMutation,
  ]);

  const autosave = useWorkflowDraftAutosave({
    enabled: workflow !== null && previewedVersion === null,
    save: persistDraft,
  });

  /** Switches the active workflow after flushing any pending draft write. */
  async function selectWorkflow(workflowId: string): Promise<void> {
    if (workflowId === resolvedWorkflowId) {
      return;
    }
    const saved = await autosave.flush({ force: true });
    if (!saved) {
      // Stay on the current draft so a failed write cannot drop unsaved edits.
      return;
    }
    setPreviewedVersion(null);
    setSelectedWorkflowId(workflowId);
    setManagerError(null);
  }

  /** Creates a persisted workflow and immediately opens it for editing. */
  async function createWorkflow(name: string): Promise<boolean> {
    setManagerError(null);
    try {
      const saved = await autosave.flush({ force: true });
      if (!saved) {
        return false;
      }
      const result = await createWorkflowMutation.mutateAsync({ name });
      // Skip selectWorkflow's forced flush — the previous draft was already written
      // above, and flushing again would race the newly created workflow's hydrate.
      autosave.cancel();
      setPreviewedVersion(null);
      setHydratedWorkflowId(null);
      setWorkflow(null);
      setSelectedWorkflowId(result.workflow.id);
      return true;
    } catch (cause) {
      setManagerError(localizeContractError(cause, t));
      return false;
    }
  }

  /** Saves pending edits, then duplicates the source workflow's current persisted draft. */
  async function copyWorkflow(workflowId: string): Promise<boolean> {
    setManagerError(null);
    try {
      const saved = await autosave.flush({ force: true });
      if (!saved) {
        return false;
      }
      const result = await copyWorkflowMutation.mutateAsync({
        workflowId,
        copyName: (name) => t("settings.workflow.copyName", { name }),
      });
      autosave.cancel();
      setPreviewedVersion(null);
      setSelectedWorkflowId(result.workflow.id);
      toast.success(
        t("settings.workflow.copySuccess", { name: result.workflow.name }),
      );
      return true;
    } catch (cause) {
      setManagerError(localizeContractError(cause, t));
      return false;
    }
  }

  /** Renames one persisted workflow. */
  async function renameWorkflow(
    workflowId: string,
    name: string,
  ): Promise<boolean> {
    const nextName = name.trim();
    if (nextName === "") {
      return false;
    }
    setManagerError(null);
    try {
      await renameWorkflowMutation.mutateAsync({ workflowId, name: nextName });
      persistedNameRef.current = nextName;
      setWorkflow((current) =>
        current === null || current.id !== workflowId
          ? current
          : { ...current, name: nextName },
      );
      return true;
    } catch (cause) {
      setManagerError(localizeContractError(cause, t));
      return false;
    }
  }

  /** Soft-deletes a workflow and lets the library effect pick the next selection. */
  async function deleteWorkflow(workflowId: string): Promise<void> {
    setManagerError(null);
    const deletedName =
      (workflow?.id === workflowId
        ? workflow.name
        : library.data?.find((item) => item.id === workflowId)?.name) ??
      workflowId;
    try {
      if (resolvedWorkflowId === workflowId) {
        autosave.cancel();
      }
      await deleteWorkflowMutation.mutateAsync(workflowId);
      if (resolvedWorkflowId === workflowId) {
        setSelectedWorkflowId(null);
        setHydratedWorkflowId(null);
        setWorkflow(null);
      }
      toast.success(
        t("settings.workflow.deleteSuccess", { name: deletedName }),
      );
    } catch (cause) {
      setManagerError(localizeContractError(cause, t));
    }
  }

  /** Immediately persists the current draft, including viewport-only changes. */
  async function saveWorkflow(): Promise<boolean> {
    return autosave.flush({ force: true });
  }

  /** Saves the draft then opens the publish dialog so a publish is never stale. */
  async function openPublishDialog(): Promise<void> {
    // Preview shows a frozen snapshot; flushing now would write that graph over the draft.
    if (previewedVersionRef.current !== null) {
      setPreviewedVersion(null);
      setPublishVersionName("");
      setPublishDialogOpen(true);
      return;
    }
    const saved = await saveWorkflow();
    if (!saved) {
      return;
    }
    setPublishVersionName("");
    setPublishDialogOpen(true);
  }

  /** Publishes the current draft as an immutable snapshot with an optional version name. */
  async function confirmPublish(): Promise<void> {
    if (workflow === null) {
      return;
    }
    setPublishDialogOpen(false);
    setManagerError(null);
    const version = publishVersionName.trim();
    try {
      const result = await publishWorkflowMutation.mutateAsync({
        workflowId: workflow.id,
        version: version === "" ? null : version,
      });
      toast.success(
        t("settings.workflow.publishSuccess", {
          version: result.snapshot.version,
        }),
      );
    } catch (cause) {
      setManagerError(localizeContractError(cause, t));
    }
  }

  /** Parses and validates an exported workflow before persisting it as a new workflow. */
  async function importWorkflow(file: File): Promise<boolean> {
    setManagerError(null);
    let imported: DemoWorkflow;
    try {
      imported = JSON.parse(await file.text()) as DemoWorkflow;
    } catch {
      setManagerError(t("settings.workflow.importError"));
      return false;
    }
    const name = imported.name.trim();
    if (name === "") {
      setManagerError(t("settings.workflow.importError"));
      return false;
    }
    const saved = await autosave.flush({ force: true });
    if (!saved) {
      return false;
    }
    try {
      const definition = normalizeWorkflowDefinition({
        id: imported.id,
        name: imported.name,
        description: imported.description,
        updatedAt: imported.updatedAt,
        viewport: imported.viewport,
        nodes: imported.nodes,
        edges: imported.edges,
      });
      const result = await createWorkflowMutation.mutateAsync({
        name,
        graph: serializeWorkflowGraph({
          nodes: definition.nodes,
          edges: definition.edges,
          viewport: definition.viewport,
          annotations: imported.annotations ?? [],
          description: definition.description,
        }),
      });
      autosave.cancel();
      setPreviewedVersion(null);
      setHydratedWorkflowId(null);
      setWorkflow(null);
      setSelectedWorkflowId(result.workflow.id);
      // Import should leave a runnable published snapshot, not only an editable draft.
      const published = await publishWorkflowMutation.mutateAsync({
        workflowId: result.workflow.id,
        version: importPublishVersion(file.name, name),
      });
      toast.success(
        t("settings.workflow.importPublishSuccess", {
          name,
          version: published.snapshot.version,
        }),
      );
      return true;
    } catch (cause) {
      setManagerError(localizeContractError(cause, t));
      return false;
    }
  }

  /** Serializes the live React Flow snapshot and sends it through the host save flow. */
  async function exportWorkflow(): Promise<void> {
    const snapshot = commitCurrentWorkflowSnapshot();
    if (snapshot === null) {
      return;
    }
    setManagerError(null);
    try {
      await platform.saveTextFile({
        defaultFileName: workflowExportFileName(snapshot.name),
        content: `${JSON.stringify(snapshot, null, 2)}\n`,
      });
    } catch {
      setManagerError(t("settings.workflow.exportError"));
    }
  }

  /** Opens a published graph in a read-only preview without mutating the editable draft. */
  async function previewWorkflowVersion(
    version: MockWorkflowVersion | null,
  ): Promise<void> {
    // Persist pending edits before leaving the editable draft; preview disables autosave.
    const saved = await autosave.flush({ force: true });
    if (!saved) {
      return;
    }
    if (version === null || resolvedWorkflowId === null) {
      setPreviewedVersion(version);
      return;
    }
    setManagerError(null);
    try {
      const { snapshot } = await client.workflow.getVersion({
        workflowId: resolvedWorkflowId,
        version: version.version,
      });
      const envelope = parseWorkflowGraph(snapshot.graph);
      setPreviewedVersion({
        id: version.id,
        version: version.version,
        createdAt: version.createdAt,
        graph: {
          nodes: envelope.nodes,
          edges: envelope.edges,
          viewport: envelope.viewport,
          annotations: envelope.annotations,
        },
      });
    } catch (cause) {
      setManagerError(localizeContractError(cause, t));
    }
  }

  /** Makes a published snapshot the active run target and loads its graph into the draft. */
  async function activateWorkflowVersion(
    version: MockWorkflowVersion,
  ): Promise<void> {
    if (workflow === null) {
      return;
    }
    // Activating the already-live snapshot is a no-op; keep the editor on the draft.
    if (draftQuery.data?.published?.version === version.version) {
      setPreviewedVersion(null);
      return;
    }
    const snapshotId = version.id;
    if (snapshotId === undefined || snapshotId === "") {
      setManagerError(t("settings.workflow.versionLoadError"));
      return;
    }
    setManagerError(null);
    try {
      await activateWorkflowMutation.mutateAsync({
        workflowId: workflow.id,
        snapshotId,
      });
      // Discard any pre-activate dirty flag; the synced draft remounts next.
      autosave.cancel();
      setHydratedWorkflowId(null);
      toast.success(
        t("settings.workflow.activateVersionSuccess", {
          version: version.version,
        }),
      );
    } catch (cause) {
      setManagerError(localizeContractError(cause, t));
    }
    setPreviewedVersion(null);
    animateInspectorTo(0);
  }

  /** Soft-deletes a non-active published version. */
  async function deleteWorkflowVersion(
    version: MockWorkflowVersion,
  ): Promise<void> {
    if (workflow === null) {
      return;
    }
    // The active published snapshot must stay addressable for runs; mirror backend refusal in UI.
    if (draftQuery.data?.published?.version === version.version) {
      return;
    }
    setManagerError(null);
    try {
      await deleteSnapshotMutation.mutateAsync({
        workflowId: workflow.id,
        version: version.version,
      });
      if (previewedVersion?.version === version.version) {
        setPreviewedVersion(null);
      }
      toast.success(
        t("settings.workflow.deleteVersionSuccess", {
          version: version.version,
        }),
      );
    } catch (cause) {
      setManagerError(localizeContractError(cause, t));
    }
  }

  /** Adds a catalog node at a canvas-provided position and selects it for immediate editing. */
  function addNode(kind: WorkflowNodeKind, position: XYPosition): void {
    if (
      workflow === null ||
      (kind === "start" &&
        workflow.nodes.some((node) => node.data.kind === "start"))
    ) {
      return;
    }
    const { sequence } = uniqueGraphId(kind, [
      ...workflow.nodes.map((node) => node.id),
      ...(workflow.annotations ?? []).map((node) => node.id),
      ...workflow.edges.map((edge) => edge.id),
    ]);
    const node = createMockWorkflowNode({
      kind,
      sequence,
      position,
      locale,
      agentConfig:
        kind === "agent" ? capabilities.defaultAgentConfig : undefined,
    });
    updateWorkflow((current) => ({
      ...current,
      nodes: [
        ...current.nodes.map((candidate) => ({
          ...candidate,
          selected: false,
        })),
        { ...node, selected: true },
      ],
    }));
    expandInspector();
  }

  /** Creates a selected editor note at the canvas-provided position. */
  function addAnnotation(position: XYPosition): void {
    if (workflow === null) {
      return;
    }
    const { id } = uniqueGraphId("annotation", [
      ...workflow.nodes.map((node) => node.id),
      ...(workflow.annotations ?? []).map((node) => node.id),
      ...workflow.edges.map((edge) => edge.id),
    ]);
    updateWorkflow((current) => ({
      ...current,
      nodes: current.nodes.map((node) => ({ ...node, selected: false })),
      annotations: [
        ...(current.annotations ?? []).map((annotation) => ({
          ...annotation,
          selected: false,
        })),
        {
          id,
          type: "annotation",
          position,
          width: 240,
          height: 140,
          selected: true,
          data: { text: "", theme: "yellow" },
        },
      ],
    }));
    animateInspectorTo(0);
  }

  /** Applies note text or theme changes without coupling callbacks to node.data. */
  function updateAnnotation(
    id: string,
    data: Partial<WorkflowAnnotationData>,
  ): void {
    updateWorkflow((current) => ({
      ...current,
      annotations: (current.annotations ?? []).map((annotation) =>
        annotation.id === id
          ? { ...annotation, data: { ...annotation.data, ...data } }
          : annotation,
      ),
    }));
  }

  /** Organizes executable nodes and exposes a position-only undo action. */
  function organizeNodes(): void {
    if (workflow === null) {
      return;
    }
    const previousPositions = new Map(
      workflow.nodes.map((node) => [node.id, node.position]),
    );
    updateWorkflow((current) => ({
      ...current,
      nodes: organizeWorkflowNodes(current.nodes, current.edges),
    }));
    toast.message(t("settings.workflow.organizeComplete"), {
      action: {
        label: t("settings.workflow.undoOrganize"),
        onClick: () => {
          updateWorkflow((current) => ({
            ...current,
            nodes: current.nodes.map((node) => ({
              ...node,
              position: previousPositions.get(node.id) ?? node.position,
            })),
          }));
        },
      },
    });
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
  function changeNodes(changes: NodeChange<WorkflowCanvasNode>[]): void {
    const persistable = changes.some(
      (change) => change.type !== "select" && change.type !== "dimensions",
    );
    updateWorkflow(
      (current) => {
        const nextNodes = applyNodeChanges<WorkflowCanvasNode>(changes, [
          ...current.nodes,
          ...(current.annotations ?? []),
        ]);
        return {
          ...current,
          nodes: nextNodes.filter(
            (node): node is Node<WorkflowNodeData, "workflow"> =>
              !isWorkflowAnnotationNode(node),
          ),
          annotations: nextNodes.filter(isWorkflowAnnotationNode),
        };
      },
      { persist: persistable },
    );
  }

  /** Applies React Flow edge changes directly to the active graph. */
  function changeEdges(changes: EdgeChange[]): void {
    const persistable = changes.some((change) => change.type !== "select");
    updateWorkflow(
      (current) => ({
        ...current,
        edges: applyEdgeChanges(changes, current.edges),
      }),
      { persist: persistable },
    );
  }

  /** Flushes the open draft, then returns to the parked workspace surface. */
  async function leaveEditor(): Promise<void> {
    const saved = await autosave.flush({ force: true });
    if (!saved) {
      toast.error(
        useWorkflowEditorStore.getState().managerError ??
          t("settings.workflow.saveError"),
      );
      return;
    }
    setManagerError(null);
    setWorkflowEditorOpen(false);
  }

  useLayoutEffect(() => {
    libraryActionsRef.current = {
      select: selectWorkflow,
      create: createWorkflow,
      copy: copyWorkflow,
      rename: renameWorkflow,
      delete: deleteWorkflow,
      importFile: importWorkflow,
      leave: leaveEditor,
    };
  });

  useLayoutEffect(() => {
    registerActions({
      select: (workflowId) =>
        libraryActionsRef.current?.select(workflowId) ?? Promise.resolve(),
      create: (name) =>
        libraryActionsRef.current?.create(name) ?? Promise.resolve(false),
      copy: (workflowId) =>
        libraryActionsRef.current?.copy(workflowId) ?? Promise.resolve(false),
      rename: (workflowId, name) =>
        libraryActionsRef.current?.rename(workflowId, name) ??
        Promise.resolve(false),
      delete: (workflowId) =>
        libraryActionsRef.current?.delete(workflowId) ?? Promise.resolve(),
      importFile: (file) =>
        libraryActionsRef.current?.importFile(file) ?? Promise.resolve(false),
      leave: () => libraryActionsRef.current?.leave() ?? Promise.resolve(),
    });
    return () => {
      registerActions(null);
    };
  }, [registerActions]);

  return (
    <main
      id="main-content"
      className="flex h-full min-h-0 min-w-0 flex-1 flex-col bg-background"
      onKeyDown={(event) => {
        if (
          event.key === "Escape" &&
          !event.defaultPrevented &&
          selectedNode !== null
        ) {
          event.preventDefault();
          event.stopPropagation();
          closeNodeInspector();
        }
      }}
    >
      <header className="flex h-14 shrink-0 items-center gap-2 border-b border-border px-3">
        {sidebarCollapsed && (
          <>
            <Button
              variant="ghost"
              size="icon"
              onClick={() => setSidebarCollapsed(false)}
              aria-label={t("sidebar.expand")}
            >
              <IconLayoutSidebarLeftExpand />
            </Button>
            {/* Chrome (show the rail) vs navigation (leave the editor) are different
                actions; keep them adjacent for reach, but split the clusters. */}
            <div className="h-4 w-px shrink-0 bg-border" aria-hidden />
            <Button
              variant="ghost"
              size="icon"
              onClick={() => void leaveEditor()}
              aria-label={t("sidebar.back")}
            >
              <IconArrowLeft />
            </Button>
          </>
        )}
        {workflow === null ? (
          <DragRegion>
            <h2 className="truncate text-sm font-medium tracking-[-0.01em]">
              {t("sidebar.workflows")}
            </h2>
          </DragRegion>
        ) : (
          <>
            <div className="min-w-0">
              <Input
                value={workflow.name}
                disabled={previewedVersion !== null}
                onChange={(event) =>
                  updateWorkflow((current) => ({
                    ...current,
                    name: event.target.value,
                  }))
                }
                aria-label={t("settings.workflow.workflowName")}
                className="h-7 max-w-72 border-transparent bg-transparent px-1 text-sm font-medium shadow-none hover:border-border focus-visible:border-border"
              />
              {workflow.description !== "" && (
                <p className="truncate px-1 text-[11px] text-muted-foreground">
                  {workflow.description}
                </p>
              )}
            </div>
            <DragRegion />
          </>
        )}
        <div className="flex shrink-0 items-center gap-2">
          {workflow !== null && previewedVersion === null && (
            <WorkflowDraftSaveStatusLabel
              status={autosave.status}
              draftUpdatedAt={draftUpdatedAt}
            />
          )}
          <Button
            variant="outline"
            size="sm"
            disabled={workflow === null}
            onClick={() => void exportWorkflow()}
          >
            <IconDownload />
            {t("settings.workflow.exportWorkflow")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={workflow === null}
            onClick={() => void openPublishDialog()}
          >
            <IconVersions />
            {t("settings.workflow.publish")}
          </Button>
          <WindowControls />
        </div>
      </header>
      {sidebarCollapsed && managerError !== null && (
        <p
          role="alert"
          className="border-b border-destructive/20 bg-destructive/10 px-3 py-2 text-xs text-destructive"
        >
          {managerError}
        </p>
      )}
      <div className="min-h-0 flex-1">
        <ResizablePanelGroup
          orientation="horizontal"
          resizeTargetMinimumSize={{ coarse: 28, fine: 12 }}
          onLayoutChanged={(_layout, meta) => {
            if (meta.isUserInteraction) {
              settleInspectorAfterUserResize();
            }
          }}
        >
          <ResizablePanel
            id="workflow-canvas"
            minSize={MIN_WORKFLOW_CANVAS_WIDTH}
          >
            {displayedWorkflow !== null ? (
              <WorkflowCanvas
                key={displayedWorkflow.id}
                capabilities={capabilities}
                nodes={displayedWorkflow.nodes}
                annotations={displayedWorkflow.annotations ?? []}
                edges={displayedWorkflow.edges}
                initialViewport={displayedWorkflow.viewport}
                onNodesChange={changeNodes}
                onEdgesChange={changeEdges}
                onAddNode={addNode}
                onAddAnnotation={addAnnotation}
                onUpdateAnnotation={updateAnnotation}
                onOrganize={organizeNodes}
                onConnect={connectNodes}
                onReconnect={reconnectEdge}
                inspectorCollapsed={inspectorCollapsed}
                inspectorAvailable={inspectorAvailable}
                onExpandInspector={expandInspector}
                versionHistory={versionHistory}
                previewedVersion={previewedVersion}
                activeVersion={draftQuery.data?.published?.version ?? null}
                draftUpdatedAt={draftUpdatedAt}
                onPreviewVersion={(version) =>
                  void previewWorkflowVersion(version)
                }
                onActivateVersion={(version) =>
                  void activateWorkflowVersion(version)
                }
                onPublishDraft={() => void openPublishDialog()}
                onDeleteVersion={(version) =>
                  void deleteWorkflowVersion(version)
                }
                readOnly={previewedVersion !== null}
              />
            ) : loadCause !== null ? (
              <WorkflowLoadError
                message={localizeContractError(loadCause, t)}
                onRetry={() => {
                  if (library.error !== null) {
                    void library.refetch();
                    return;
                  }
                  void draftQuery.refetch();
                }}
              />
            ) : library.data !== undefined && library.data.length === 0 ? (
              <WorkflowEmpty
                onCreate={() =>
                  void createWorkflow(
                    t("settings.workflow.untitledWorkflow", {
                      count: 1,
                    }),
                  )
                }
              />
            ) : (
              <WorkflowLoading />
            )}
          </ResizablePanel>
          <ResizableHandle
            withHandle
            aria-label={t("settings.workflow.resizeConfiguration")}
            title={t("settings.workflow.resizeConfiguration")}
            className="z-20 after:w-3 transition-colors hover:bg-ring focus-visible:bg-ring"
            onPointerDown={() =>
              cancelWorkflowPanelAnimation(inspectorAnimationRef)
            }
            onDoubleClick={() => {
              inspectorWidthRef.current = DEFAULT_WORKFLOW_INSPECTOR_WIDTH;
              inspectorPanelRef.current?.resize(
                DEFAULT_WORKFLOW_INSPECTOR_WIDTH,
              );
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
                    (inspectorVisualWidth - WORKFLOW_INSPECTOR_FADE_START) /
                      (MIN_WORKFLOW_INSPECTOR_WIDTH -
                        WORKFLOW_INSPECTOR_FADE_START),
                  ),
                ),
              }}
            >
              <WorkflowInspector
                node={selectedNode}
                capabilities={capabilities}
                agentModelsLoading={agentModelsLoading}
                agentModelsError={agentModelsError}
                onRetryAgentModels={agentModelsCatalog.refetch}
                agents={agentModelsCatalog.agents}
                modelsByCli={agentModelsCatalog.modelsByCli}
                cliStatus={agentModelsCatalog.cliStatus}
                agentCatalogsLoading={agentCatalogsLoading}
                agentCatalogsError={agentCatalogsError}
                onRetryAgentCatalogs={() => {
                  void agentsQuery.refetch();
                  void skillsQuery.refetch();
                }}
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
              />
            </div>
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>
      <AlertDialog open={publishDialogOpen} onOpenChange={setPublishDialogOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.workflow.publishTitle")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.workflow.publishDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <Input
            value={publishVersionName}
            onChange={(event) => setPublishVersionName(event.target.value)}
            aria-label={t("settings.workflow.versionHistory")}
            placeholder={t("settings.workflow.publishVersionPlaceholder")}
            autoFocus
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void confirmPublish();
              }
            }}
          />
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void confirmPublish()}>
              <IconVersions />
              {t("settings.workflow.publish")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </main>
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
        <h3 className="mt-3 text-sm font-semibold">
          {t("settings.workflow.emptyTitle")}
        </h3>
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

/** Distinguishes a hydrating library/draft from a truly empty collection. */
function WorkflowLoading() {
  const { t } = useTranslation();
  return (
    <section
      className="flex min-h-0 flex-1 items-center justify-center bg-muted/25"
      aria-busy="true"
      aria-live="polite"
    >
      <p className="text-sm text-muted-foreground">
        {t("settings.workflow.loading")}
      </p>
    </section>
  );
}

/** Failed library or draft fetch: a retryable error, not an empty collection or spinner. */
function WorkflowLoadError({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => void;
}) {
  const { t } = useTranslation();
  return (
    <section
      className="flex min-h-0 flex-1 items-center justify-center bg-muted/25"
      role="alert"
    >
      <div className="max-w-64 text-center">
        <h3 className="text-sm font-semibold">
          {t("settings.workflow.loadError")}
        </h3>
        <p className="mt-1 text-[11px] leading-4 text-muted-foreground">
          {message}
        </p>
        <Button size="sm" className="mt-4" onClick={onRetry}>
          {t("common.retry")}
        </Button>
      </div>
    </section>
  );
}
