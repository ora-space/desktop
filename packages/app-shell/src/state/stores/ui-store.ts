import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { Session } from "@ora/contracts";
import { createDebouncedJSONStorage } from "./debounced-json-storage";

/** Shape of the create dialog currently driven from the workspace tree. */
export type DialogState =
  | { kind: "project" }
  | { kind: "task"; projectId: string }
  | { kind: "session"; workspaceId: string; entity?: Session }
  | {
      kind: "runWorkflow";
      projectId: string;
      workspaceId: string;
      taskId?: string;
      workflowId: string;
      workflowName: string;
    };

/** Shape of the delete-confirmation dialog driven from the workspace tree. */
export type DeleteTarget =
  | { kind: "project"; id: string; name: string; sessionIds: string[] }
  | {
      kind: "task";
      id: string;
      name: string;
      sessionIds: string[];
    }
  | { kind: "session"; id: string; name: string }
  | { kind: "workflowRun"; id: string; name: string; projectId: string };

export const UI_STORAGE_KEY = "ora.ui.v1";

/**
 * A one-shot destination inside the plugins pane requested by another surface: a
 * marketplace search, the installed-plugin manager, or one plugin's configuration editor.
 */
export type PluginSettingsRequest =
  | { kind: "marketplaceSearch"; query: string }
  | { kind: "manage" }
  | { kind: "configure"; pluginId: string; displayName: string };

/**
 * Sidebar workflow-run filter. `awaiting_input` is the Theater display spelling
 * so HITL rows match the status dots rather than the wire `awaitingInput` token.
 */
export type SidebarWorkflowRunStatusFilter =
  | "all"
  | "pending"
  | "running"
  | "awaiting_input"
  | "succeeded"
  | "failed"
  | "cancelled";

const SIDEBAR_WORKFLOW_RUN_STATUS_FILTERS = new Set<string>([
  "all",
  "pending",
  "running",
  "awaiting_input",
  "succeeded",
  "failed",
  "cancelled",
]);

/** The settings categories the dialog can be asked to open on. */
export type SettingsCategory =
  | "appearance"
  | "roles"
  | "skills"
  | "plugins"
  | "proxy"
  | "privacy"
  | "developer";

interface UiState {
  sidebarCollapsed: boolean;
  settingsOpen: boolean;
  /**
   * The settings category currently shown, and the one the dialog opens on by
   * default. Held here rather than in the dialog so other surfaces can deep-link
   * to a specific category through {@link openSettingsAt}; it is transient and
   * never persisted.
   */
  settingsCategory: SettingsCategory;
  setSettingsCategory(category: SettingsCategory): void;
  /** Requests the category the settings dialog should open on, and opens it. */
  openSettingsAt(category: SettingsCategory): void;
  /**
   * Pending deep link into the plugins pane (for example from a workflow dependency).
   * The pane adopts it on mount and clears it, so later visits start from the default view.
   */
  pluginSettingsRequest: PluginSettingsRequest | null;
  /** Opens Settings on the plugins pane at the requested destination. */
  openPluginSettings(request: PluginSettingsRequest): void;
  clearPluginSettingsRequest(): void;
  /** First-class workflow definition editor; session-only, not persisted. */
  workflowEditorOpen: boolean;
  expandedProjects: Set<string>;
  expandedTasks: Set<string>;
  /**
   * True after the first-run expand-all seed, or after the user toggles a row.
   * Returning sessions must trust the persisted expand sets verbatim.
   */
  treeExpansionBootstrapped: boolean;
  /**
   * Per-project workflow status filter. Missing keys mean All. Survives restart
   * so a HITL-focused project does not reset every launch.
   */
  workflowRunStatusFilterByProjectId: Record<
    string,
    SidebarWorkflowRunStatusFilter
  >;
  dialog: DialogState | null;
  deleteTarget: DeleteTarget | null;
  setSidebarCollapsed: (collapsed: boolean) => void;
  setSettingsOpen: (open: boolean) => void;
  setWorkflowEditorOpen: (open: boolean) => void;
  toggleProjectExpand: (projectId: string) => void;
  toggleTaskExpand: (taskId: string) => void;
  /** Expands a project without toggling it closed (used after mutations select a child). */
  expandProject: (projectId: string) => void;
  /** Expands a task without toggling it closed (used after mutations select a child). */
  expandTask: (taskId: string) => void;
  /**
   * First-run only: expands every known project/task and marks expansion seeded
   * so later restarts trust the persisted sets instead of opening the whole tree.
   */
  bootstrapTreeExpansion: (
    projectIds: readonly string[],
    taskIds: readonly string[],
  ) => void;
  /**
   * Drops expand ids that no longer exist in the live tree so deleted rows do
   * not accumulate forever on disk.
   */
  pruneTreeExpansion: (
    projectIds: readonly string[],
    taskIds: readonly string[],
  ) => void;
  /** Persists the sidebar workflow status filter for one project. `all` drops the key. */
  setWorkflowRunStatusFilter: (
    projectId: string,
    filter: SidebarWorkflowRunStatusFilter,
  ) => void;
  setDialog: (dialog: DialogState | null) => void;
  setDeleteTarget: (target: DeleteTarget | null) => void;
}

/** Disk shape for the UI slice — Sets become string arrays for JSON. */
interface UiPersistSlice {
  sidebarCollapsed?: unknown;
  expandedProjects?: unknown;
  expandedTasks?: unknown;
  treeExpansionBootstrapped?: unknown;
  workflowRunStatusFilterByProjectId?: unknown;
}

/** Layout fields restored from disk (or defaults when missing/corrupt). */
export interface UiPersistFields {
  sidebarCollapsed: boolean;
  expandedProjects: Set<string>;
  expandedTasks: Set<string>;
  treeExpansionBootstrapped: boolean;
  workflowRunStatusFilterByProjectId: Record<
    string,
    SidebarWorkflowRunStatusFilter
  >;
}

/** Drops corrupt project ids and unknown status tokens from a persist payload. */
function sanitizeStatusFilterMap(
  value: unknown,
): Record<string, SidebarWorkflowRunStatusFilter> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return {};
  }
  const next: Record<string, SidebarWorkflowRunStatusFilter> = {};
  for (const [projectId, filter] of Object.entries(value)) {
    if (projectId.length === 0) continue;
    if (
      typeof filter === "string" &&
      SIDEBAR_WORKFLOW_RUN_STATUS_FILTERS.has(filter)
    ) {
      next[projectId] = filter as SidebarWorkflowRunStatusFilter;
    }
  }
  return next;
}

/** Keeps only non-empty string ids so corrupt disk payloads cannot poison the tree. */
function sanitizeIdSet(value: unknown): Set<string> {
  if (!Array.isArray(value)) return new Set();
  return new Set(
    value.filter((id): id is string => typeof id === "string" && id.length > 0),
  );
}

/** Maps an untrusted persist slice onto the layout fields the store owns. */
export function sanitizeUiPersistSlice(
  slice: UiPersistSlice | undefined,
): UiPersistFields {
  if (slice === undefined) {
    return {
      sidebarCollapsed: false,
      expandedProjects: new Set(),
      expandedTasks: new Set(),
      treeExpansionBootstrapped: false,
      workflowRunStatusFilterByProjectId: {},
    };
  }
  return {
    sidebarCollapsed: slice.sidebarCollapsed === true,
    expandedProjects: sanitizeIdSet(slice.expandedProjects),
    expandedTasks: sanitizeIdSet(slice.expandedTasks),
    treeExpansionBootstrapped: slice.treeExpansionBootstrapped === true,
    workflowRunStatusFilterByProjectId: sanitizeStatusFilterMap(
      slice.workflowRunStatusFilterByProjectId,
    ),
  };
}

/**
 * Reads `ora.ui.v1` synchronously so the first paint already matches the last
 * closed layout. Async rehydrate still runs afterward and must agree.
 */
export function readUiPersistFromDisk(): UiPersistFields {
  if (typeof window === "undefined") return sanitizeUiPersistSlice(undefined);
  try {
    const raw = window.localStorage.getItem(UI_STORAGE_KEY);
    if (raw === null) return sanitizeUiPersistSlice(undefined);
    const parsed = JSON.parse(raw) as { state?: UiPersistSlice };
    return sanitizeUiPersistSlice(
      typeof parsed.state === "object" && parsed.state !== null
        ? parsed.state
        : undefined,
    );
  } catch {
    return sanitizeUiPersistSlice(undefined);
  }
}

const initialPersist = readUiPersistFromDisk();

/**
 * Global UI state for the app shell: sidebar folding, tree expansion, and dialog
 * switches. Layout preferences that should survive restart are mirrored to
 * localStorage; transient dialogs and open panels stay memory-only.
 */
export const useUiStore = create<UiState>()(
  persist(
    (set, get) => ({
      sidebarCollapsed: initialPersist.sidebarCollapsed,
      settingsOpen: false,
      settingsCategory: "appearance",
      pluginSettingsRequest: null,
      workflowEditorOpen: false,
      expandedProjects: initialPersist.expandedProjects,
      expandedTasks: initialPersist.expandedTasks,
      treeExpansionBootstrapped: initialPersist.treeExpansionBootstrapped,
      workflowRunStatusFilterByProjectId:
        initialPersist.workflowRunStatusFilterByProjectId,
      dialog: null,
      deleteTarget: null,
      setSidebarCollapsed: (sidebarCollapsed) => set({ sidebarCollapsed }),
      setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
      setSettingsCategory: (settingsCategory) => set({ settingsCategory }),
      openSettingsAt: (settingsCategory) =>
        set({ settingsOpen: true, settingsCategory }),
      openPluginSettings: (pluginSettingsRequest) =>
        set({
          settingsOpen: true,
          settingsCategory: "plugins",
          pluginSettingsRequest,
        }),
      clearPluginSettingsRequest: () => set({ pluginSettingsRequest: null }),
      setWorkflowEditorOpen: (workflowEditorOpen) =>
        set({ workflowEditorOpen }),
      toggleProjectExpand: (projectId) =>
        set((state) => {
          const next = new Set(state.expandedProjects);
          if (next.has(projectId)) next.delete(projectId);
          else next.add(projectId);
          // A manual toggle leaves first-run defaults; never expand-all after this.
          return {
            expandedProjects: next,
            treeExpansionBootstrapped: true,
          };
        }),
      toggleTaskExpand: (taskId) =>
        set((state) => {
          const next = new Set(state.expandedTasks);
          if (next.has(taskId)) next.delete(taskId);
          else next.add(taskId);
          return { expandedTasks: next, treeExpansionBootstrapped: true };
        }),
      expandProject: (projectId) =>
        set((state) =>
          state.expandedProjects.has(projectId)
            ? state
            : {
                expandedProjects: new Set(state.expandedProjects).add(
                  projectId,
                ),
              },
        ),
      expandTask: (taskId) =>
        set((state) =>
          state.expandedTasks.has(taskId)
            ? state
            : { expandedTasks: new Set(state.expandedTasks).add(taskId) },
        ),
      bootstrapTreeExpansion: (projectIds, taskIds) => {
        if (get().treeExpansionBootstrapped) return;
        set((state) => ({
          expandedProjects: new Set([...state.expandedProjects, ...projectIds]),
          expandedTasks: new Set([...state.expandedTasks, ...taskIds]),
          treeExpansionBootstrapped: true,
        }));
      },
      pruneTreeExpansion: (projectIds, taskIds) =>
        set((state) => {
          const liveProjects = new Set(projectIds);
          const liveTasks = new Set(taskIds);
          const expandedProjects = new Set(
            [...state.expandedProjects].filter((id) => liveProjects.has(id)),
          );
          const expandedTasks = new Set(
            [...state.expandedTasks].filter((id) => liveTasks.has(id)),
          );
          const workflowRunStatusFilterByProjectId = Object.fromEntries(
            Object.entries(state.workflowRunStatusFilterByProjectId).filter(
              ([id]) => liveProjects.has(id),
            ),
          );
          if (
            expandedProjects.size === state.expandedProjects.size &&
            expandedTasks.size === state.expandedTasks.size &&
            Object.keys(workflowRunStatusFilterByProjectId).length ===
              Object.keys(state.workflowRunStatusFilterByProjectId).length
          ) {
            return state;
          }
          return {
            expandedProjects,
            expandedTasks,
            workflowRunStatusFilterByProjectId,
          };
        }),
      setWorkflowRunStatusFilter: (projectId, filter) =>
        set((state) => {
          const current =
            state.workflowRunStatusFilterByProjectId[projectId] ?? "all";
          if (current === filter) return state;
          const next = { ...state.workflowRunStatusFilterByProjectId };
          if (filter === "all") delete next[projectId];
          else next[projectId] = filter;
          return { workflowRunStatusFilterByProjectId: next };
        }),
      setDialog: (dialog) => set({ dialog }),
      setDeleteTarget: (deleteTarget) => set({ deleteTarget }),
    }),
    {
      name: UI_STORAGE_KEY,
      storage: createDebouncedJSONStorage(),
      partialize: (state) => ({
        sidebarCollapsed: state.sidebarCollapsed,
        expandedProjects: [...state.expandedProjects],
        expandedTasks: [...state.expandedTasks],
        treeExpansionBootstrapped: state.treeExpansionBootstrapped,
        workflowRunStatusFilterByProjectId:
          state.workflowRunStatusFilterByProjectId,
      }),
      merge: (persisted, current) => {
        const slice =
          typeof persisted === "object" && persisted !== null
            ? (persisted as UiPersistSlice)
            : undefined;
        const restored = sanitizeUiPersistSlice(slice);
        return {
          ...current,
          ...restored,
          expandedProjects: current.treeExpansionBootstrapped
            ? current.expandedProjects
            : restored.expandedProjects,
          expandedTasks: current.treeExpansionBootstrapped
            ? current.expandedTasks
            : restored.expandedTasks,
          treeExpansionBootstrapped:
            current.treeExpansionBootstrapped ||
            restored.treeExpansionBootstrapped,
          sidebarCollapsed:
            current.sidebarCollapsed !== initialPersist.sidebarCollapsed
              ? current.sidebarCollapsed
              : restored.sidebarCollapsed,
          workflowRunStatusFilterByProjectId:
            Object.keys(current.workflowRunStatusFilterByProjectId ?? {})
              .length > 0
              ? current.workflowRunStatusFilterByProjectId
              : restored.workflowRunStatusFilterByProjectId,
        };
      },
    },
  ),
);
