import type {
  Agent,
  ContractsClient,
  Project,
  Session,
  SessionStatus,
  Skill,
  Task,
  TaskStatus,
} from "@ora/contracts";

/** In-memory state mutated by the mock client so tests can assert post-call state. */
export interface MockClientState {
  projects: Project[];
  tasks: Task[];
  sessions: Session[];
  agents: Agent[];
  skills: Skill[];
}

/** Creates a fresh in-memory mock state with no records. */
export function createMockClientState(): MockClientState {
  return { projects: [], tasks: [], sessions: [], agents: [], skills: [] };
}

function nextId(prefix: string, count: number): string {
  return `${prefix}${count + 1}`;
}

/**
 * Builds a ContractsClient whose CRUD operations mutate the supplied state arrays.
 * Mirrors the real client surface so react-query hooks exercise the same code path.
 */
export function createMockClient(state: MockClientState): ContractsClient {
  return {
    project: {
      list: async () => ({ projects: [...state.projects] }),
      get: async (req) => ({ project: state.projects.find((p) => p.id === req.projectId)! }),
      create: async (req) => {
        const project: Project = { id: nextId("p", state.projects.length), name: req.name, rootPath: req.rootPath };
        state.projects.push(project);
        return { project };
      },
      update: async (req) => {
        const idx = state.projects.findIndex((p) => p.id === req.projectId);
        if (idx < 0) throw new Error(`project ${req.projectId} not found`);
        const updated: Project = { id: req.projectId, name: req.name, rootPath: req.rootPath };
        state.projects[idx] = updated;
        return { project: updated };
      },
      delete: async (req) => {
        const idx = state.projects.findIndex((p) => p.id === req.projectId);
        if (idx >= 0) state.projects.splice(idx, 1);
        return { projectId: req.projectId };
      },
    },
    projectWorkContext: {
      open: async () => { throw new Error("projectWorkContext not implemented in mock"); },
      renew: async () => { throw new Error("projectWorkContext not implemented in mock"); },
    },
    task: {
      list: async () => ({ tasks: [...state.tasks] }),
      get: async (req) => ({ task: state.tasks.find((t) => t.id === req.taskId)! }),
      create: async (req) => {
        const task: Task = {
          id: nextId("t", state.tasks.length),
          projectId: req.projectId,
          title: req.title,
          status: req.status as TaskStatus,
        };
        state.tasks.push(task);
        return { task };
      },
      update: async (req) => {
        const idx = state.tasks.findIndex((t) => t.id === req.taskId);
        if (idx < 0) throw new Error(`task ${req.taskId} not found`);
        const updated: Task = {
          id: req.taskId,
          projectId: req.projectId,
          title: req.title,
          status: req.status as TaskStatus,
        };
        state.tasks[idx] = updated;
        return { task: updated };
      },
      delete: async (req) => {
        const idx = state.tasks.findIndex((t) => t.id === req.taskId);
        if (idx >= 0) state.tasks.splice(idx, 1);
        return { taskId: req.taskId };
      },
    },
    session: {
      list: async () => ({ sessions: [...state.sessions] }),
      get: async (req) => ({ session: state.sessions.find((s) => s.id === req.sessionId)! }),
      create: async (req) => {
        const session: Session = {
          id: nextId("s", state.sessions.length),
          taskId: req.taskId,
          agentId: req.agentId,
          agentSessionId: req.agentSessionId,
          status: req.status as SessionStatus,
        };
        state.sessions.push(session);
        return { session };
      },
      update: async (req) => {
        const idx = state.sessions.findIndex((s) => s.id === req.sessionId);
        if (idx < 0) throw new Error(`session ${req.sessionId} not found`);
        const existing = state.sessions[idx]!;
        const updated: Session = {
          id: req.sessionId,
          taskId: req.taskId,
          agentId: req.agentId,
          agentSessionId: req.agentSessionId,
          status: req.status as SessionStatus,
        };
        state.sessions[idx] = { ...existing, ...updated };
        return { session: state.sessions[idx]! };
      },
      delete: async (req) => {
        const idx = state.sessions.findIndex((s) => s.id === req.sessionId);
        if (idx >= 0) state.sessions.splice(idx, 1);
        return { sessionId: req.sessionId };
      },
    },
    agent: {
      list: async () => ({ agents: [...state.agents] }),
      get: async (req) => ({ agent: state.agents.find((a) => a.id === req.agentId)! }),
      create: async (req) => {
        const agent: Agent = { id: nextId("a", state.agents.length), name: req.name, description: req.description };
        state.agents.push(agent);
        return { agent };
      },
      update: async (req) => {
        const idx = state.agents.findIndex((a) => a.id === req.agentId);
        if (idx < 0) throw new Error(`agent ${req.agentId} not found`);
        const updated: Agent = { id: req.agentId, name: req.name, description: req.description };
        state.agents[idx] = updated;
        return { agent: updated };
      },
      delete: async (req) => {
        const idx = state.agents.findIndex((a) => a.id === req.agentId);
        if (idx >= 0) state.agents.splice(idx, 1);
        return { agentId: req.agentId };
      },
    },
    skill: {
      list: async () => ({ skills: [...state.skills] }),
      get: async (req) => ({ skill: state.skills.find((s) => s.id === req.skillId)! }),
      create: async (req) => {
        const skill: Skill = { id: nextId("sk", state.skills.length), name: req.name, description: req.description };
        state.skills.push(skill);
        return { skill };
      },
      update: async (req) => {
        const idx = state.skills.findIndex((s) => s.id === req.skillId);
        if (idx < 0) throw new Error(`skill ${req.skillId} not found`);
        const updated: Skill = { id: req.skillId, name: req.name, description: req.description };
        state.skills[idx] = updated;
        return { skill: updated };
      },
      delete: async (req) => {
        const idx = state.skills.findIndex((s) => s.id === req.skillId);
        if (idx >= 0) state.skills.splice(idx, 1);
        return { skillId: req.skillId };
      },
    },
    fileSystem: {
      listDirectory: async (request) => ({
        currentPath: request.path ?? "/home/test",
        parentPath: null,
        breadcrumbs: [],
        entries: [],
      }),
    },
  };
}
