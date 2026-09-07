import { type Project, type Task, type Workspace } from "@ora/contracts";
import type { TestHandlers } from "../contracts-transport";
import { nextId } from "./records";

/** Mutable records owned by the workspaces test adapter. */
export interface WorkspaceMemoryState {
  projects: Project[];
  workspaces: Workspace[];
  tasks: Task[];
}

/** Creates an independent workspaces memory fixture. */
export function createWorkspaceMemory(): WorkspaceMemoryState {
  return { projects: [], workspaces: [], tasks: [] };
}

/** Returns or creates the mock project's canonical Workspace projection. */
function mainWorkspaceFor(
  state: WorkspaceMemoryState,
  projectId: string,
): Workspace {
  const existing = state.workspaces.find(
    (workspace) =>
      workspace.projectId === projectId && workspace.kind === "main",
  );
  if (existing !== undefined) return existing;
  const workspace: Workspace = {
    id: `workspace-${projectId}`,
    projectId,
    kind: "main",
    lifecycle: "active",
  };
  state.workspaces.push(workspace);
  return workspace;
}

/** Returns every explicit workspace plus a stable main projection for seeded projects. */
export function visibleWorkspaces(state: WorkspaceMemoryState): Workspace[] {
  for (const project of state.projects) {
    mainWorkspaceFor(state, project.id);
  }
  return [...state.workspaces];
}

/** Registers only the workspaces operations explicitly requested by a fixture. */
export function workspaceHandlers(state: WorkspaceMemoryState) {
  return {
    listProjects: async () => ({ projects: [...state.projects] }),
    listProjectBranches: async () => ({
      branches: [{ name: "main", refName: "origin/main", displayName: "main" }],
    }),
    getProject: async (req) => ({
      project: state.projects.find((p) => p.id === req.projectId)!,
    }),
    createProject: async (req) => {
      const project: Project = {
        id: nextId("p", state.projects.length),
        name: req.name,
      };
      state.projects.push(project);
      mainWorkspaceFor(state, project.id);
      return { project };
    },
    updateProject: async (req) => {
      const idx = state.projects.findIndex((p) => p.id === req.projectId);
      if (idx < 0) throw new Error(`project ${req.projectId} not found`);
      const updated: Project = { ...state.projects[idx]!, name: req.name };
      state.projects[idx] = updated;
      return { project: updated };
    },
    deleteProject: async (req) => {
      const idx = state.projects.findIndex((p) => p.id === req.projectId);
      if (idx >= 0) state.projects.splice(idx, 1);
      return { projectId: req.projectId };
    },
    listWorkspaces: async () => ({ workspaces: visibleWorkspaces(state) }),
    getWorkspaceDiff: async () => ({
      baseCommitId: "base",
      headCommitId: "head",
      patch: "",
    }),
    listTasks: async () => ({ tasks: [...state.tasks] }),
    getTask: async (req) => ({
      task: state.tasks.find((t) => t.id === req.taskId)!,
    }),
    createTask: async (req) => {
      const task: Task = {
        id: nextId("t", state.tasks.length),
        projectId: req.projectId,
        workspaceId: `workspace-${nextId("t", state.tasks.length)}`,
        title: req.title,
      };
      state.workspaces.push({
        id: task.workspaceId,
        projectId: task.projectId,
        kind: "isolated",
        lifecycle: "active",
      });
      state.tasks.push(task);
      return { task };
    },
    updateTask: async (req) => {
      const idx = state.tasks.findIndex((t) => t.id === req.taskId);
      if (idx < 0) throw new Error(`task ${req.taskId} not found`);
      const updated: Task = {
        ...state.tasks[idx]!,
        title: req.title,
      };
      state.tasks[idx] = updated;
      return { task: updated };
    },
    deleteTask: async (req) => {
      const idx = state.tasks.findIndex((t) => t.id === req.taskId);
      const workspaceId =
        idx >= 0 ? state.tasks[idx]!.workspaceId : `workspace-${req.taskId}`;
      if (idx >= 0) state.tasks.splice(idx, 1);
      return { taskId: req.taskId, workspaceId };
    },
    getTaskWorkspace: async (req) => ({
      workspace: {
        rootPath: `/worktrees/${req.taskId}`,
        branchName: `task/${req.taskId}`,
      },
    }),
  } satisfies TestHandlers;
}
