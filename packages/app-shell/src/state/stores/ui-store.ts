import { create } from "zustand";
import type { Project, Session, Task } from "@ora/contracts";

/** Shape of the create/edit dialog currently driven from the workspace tree. */
export type DialogState =
  | { kind: "project"; entity?: Project }
  | { kind: "task"; projectId: string; entity?: Task }
  | { kind: "session"; taskId: string; entity?: Session };

/** Shape of the delete-confirmation dialog driven from the workspace tree. */
export type DeleteTarget =
  | { kind: "project"; id: string; name: string; sessionIds: string[] }
  | { kind: "task"; id: string; name: string; workspaceMode: Task["workspaceMode"]; sessionIds: string[] }
  | { kind: "session"; id: string; name: string };

interface UiState {
  sidebarCollapsed: boolean;
  settingsOpen: boolean;
  dashboardOpen: boolean;
  /** Resizable dashboard panel width in px; clamped to a sane min/max by the panel. */
  dashboardWidth: number;
  expandedProjects: Set<string>;
  expandedTasks: Set<string>;
  dialog: DialogState | null;
  deleteTarget: DeleteTarget | null;
  setSidebarCollapsed: (collapsed: boolean) => void;
  setSettingsOpen: (open: boolean) => void;
  setDashboardOpen: (open: boolean) => void;
  setDashboardWidth: (width: number) => void;
  toggleProjectExpand: (projectId: string) => void;
  toggleTaskExpand: (taskId: string) => void;
  /** Expands a project without toggling it closed (used after mutations select a child). */
  expandProject: (projectId: string) => void;
  /** Expands a task without toggling it closed (used after mutations select a child). */
  expandTask: (taskId: string) => void;
  setDialog: (dialog: DialogState | null) => void;
  setDeleteTarget: (target: DeleteTarget | null) => void;
}

/** Global UI state for the app shell: sidebar folding, tree expansion, and dialog switches. */
export const useUiStore = create<UiState>((set) => ({
  sidebarCollapsed: false,
  settingsOpen: false,
  dashboardOpen: false,
  dashboardWidth: 800,
  expandedProjects: new Set<string>(),
  expandedTasks: new Set<string>(),
  dialog: null,
  deleteTarget: null,
  setSidebarCollapsed: (sidebarCollapsed) => set({ sidebarCollapsed }),
  setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
  setDashboardOpen: (dashboardOpen) => set({ dashboardOpen }),
  setDashboardWidth: (dashboardWidth) => set({ dashboardWidth }),
  toggleProjectExpand: (projectId) =>
    set((state) => {
      const next = new Set(state.expandedProjects);
      if (next.has(projectId)) next.delete(projectId);
      else next.add(projectId);
      return { expandedProjects: next };
    }),
  toggleTaskExpand: (taskId) =>
    set((state) => {
      const next = new Set(state.expandedTasks);
      if (next.has(taskId)) next.delete(taskId);
      else next.add(taskId);
      return { expandedTasks: next };
    }),
  expandProject: (projectId) =>
    set((state) =>
      state.expandedProjects.has(projectId)
        ? state
        : { expandedProjects: new Set(state.expandedProjects).add(projectId) },
    ),
  expandTask: (taskId) =>
    set((state) =>
      state.expandedTasks.has(taskId)
        ? state
        : { expandedTasks: new Set(state.expandedTasks).add(taskId) },
    ),
  setDialog: (dialog) => set({ dialog }),
  setDeleteTarget: (deleteTarget) => set({ deleteTarget }),
}));
