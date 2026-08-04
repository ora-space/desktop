import type { Agent } from "./agent.js";
import type { ContractsClient } from "./client.js";
import type { Project } from "./project.js";
import type { ProjectWorkContext } from "./project-work-context.js";
import type { Session } from "./session.js";
import type { Skill } from "./skill.js";
import type { Task, TaskStatus } from "./task.js";

/** Mutable records owned by one in-memory ContractsClient instance. */
export interface MemoryContractsState {
  projects: Project[];
  tasks: Task[];
  sessions: Session[];
  agents: Agent[];
  skills: Skill[];
}

/** Creates isolated state, optionally seeded for a browser prototype. */
export function createMemoryContractsState(
  seed: Partial<MemoryContractsState> = {},
): MemoryContractsState {
  return {
    projects: structuredClone(seed.projects ?? []),
    tasks: structuredClone(seed.tasks ?? []),
    sessions: structuredClone(seed.sessions ?? []),
    agents: structuredClone(seed.agents ?? []),
    skills: structuredClone(seed.skills ?? []),
  };
}

/** Finds the first available deterministic identifier for a record collection. */
function nextId(prefix: string, records: readonly { id: string }[]): string {
  const existing = new Set(records.map((record) => record.id));
  let suffix = 1;
  while (existing.has(`${prefix}${suffix}`)) {
    suffix += 1;
  }
  return `${prefix}${suffix}`;
}

/**
 * Builds a stateful in-memory implementation of the generated client surface.
 * It is intended for tests and explicit prototype composition roots; production
 * Web and Desktop builds continue to use their real transports.
 */
export function createMemoryContractsClient(
  state: MemoryContractsState = createMemoryContractsState(),
): ContractsClient {
  const workContexts = new Map<string, ProjectWorkContext>();

  return {
    project: {
      list: async () => ({ projects: structuredClone(state.projects) }),
      get: async (request) => ({
        project: structuredClone(requireRecord(state.projects, request.projectId, "project")),
      }),
      create: async (request) => {
        const project: Project = {
          id: nextId("p", state.projects),
          name: request.name,
          rootPath: request.rootPath,
        };
        state.projects.push(project);
        return { project: structuredClone(project) };
      },
      update: async (request) => {
        const index = requireRecordIndex(state.projects, request.projectId, "project");
        const project: Project = { ...state.projects[index]!, name: request.name };
        state.projects[index] = project;
        return { project: structuredClone(project) };
      },
      delete: async (request) => {
        removeRecord(state.projects, request.projectId);
        return { projectId: request.projectId };
      },
    },
    projectWorkContext: {
      open: async (request) => {
        requireRecord(state.projects, request.projectId, "project");
        const key = `${request.surface}:${request.windowId}`;
        const context: ProjectWorkContext = {
          id: `mock-context:${key}`,
          surface: request.surface,
          windowId: request.windowId,
          projectId: request.projectId,
          leaseExpiresAt: Date.now() + 30_000,
        };
        workContexts.set(key, context);
        return { context: structuredClone(context) };
      },
      renew: async (request) => {
        const key = `${request.surface}:${request.windowId}`;
        const current = workContexts.get(key);
        if (current === undefined) {
          throw new Error(`project work context ${key} not found`);
        }
        const context = { ...current, leaseExpiresAt: Date.now() + 30_000 };
        workContexts.set(key, context);
        return { context: structuredClone(context) };
      },
    },
    task: {
      list: async () => ({ tasks: structuredClone(state.tasks) }),
      get: async (request) => ({
        task: structuredClone(requireRecord(state.tasks, request.taskId, "task")),
      }),
      create: async (request) => {
        requireRecord(state.projects, request.projectId, "project");
        const task: Task = {
          id: nextId("t", state.tasks),
          projectId: request.projectId,
          title: request.title,
          status: request.status as TaskStatus,
          workspaceMode: request.workspaceMode ?? "worktree",
        };
        state.tasks.push(task);
        return { task: structuredClone(task) };
      },
      update: async (request) => {
        const index = requireRecordIndex(state.tasks, request.taskId, "task");
        const task: Task = {
          ...state.tasks[index]!,
          title: request.title,
          status: request.status as TaskStatus,
        };
        state.tasks[index] = task;
        return { task: structuredClone(task) };
      },
      delete: async (request) => {
        removeRecord(state.tasks, request.taskId);
        return { taskId: request.taskId };
      },
    },
    session: {
      list: async () => ({ sessions: structuredClone(state.sessions) }),
      get: async (request) => ({
        session: structuredClone(requireRecord(state.sessions, request.sessionId, "session")),
      }),
      create: async (request) => {
        requireRecord(state.tasks, request.taskId, "task");
        const session: Session = {
          id: nextId("s", state.sessions),
          taskId: request.taskId,
          agentCli: request.agentCli,
          status: "running",
        };
        state.sessions.push(session);
        return { session: structuredClone(session), availableCommands: [] };
      },
      load: async function* () {
        yield { type: "completed" as const };
      },
      prompt: async function* () {
        yield { type: "completed" as const, stopReason: "end_turn" as const };
      },
      respondToPermission: async () => ({}),
      stop: async (request) => {
        const index = requireRecordIndex(state.sessions, request.sessionId, "session");
        const session: Session = { ...state.sessions[index]!, status: "stopped" };
        state.sessions[index] = session;
        return { session: structuredClone(session) };
      },
      delete: async (request) => {
        removeRecord(state.sessions, request.sessionId);
        return { sessionId: request.sessionId };
      },
    },
    agentRuntime: {
      listModels: async () => ({
        groups: [
          { agentCli: "open_code", models: ["opencode/big-pickle", "opencode/small-pickle"] },
          { agentCli: "nga", models: ["nga/default"] },
          { agentCli: "code_agent_cli", models: ["codeagentcli/default"] },
        ],
      }),
    },
    agent: {
      list: async () => ({ agents: structuredClone(state.agents) }),
      get: async (request) => ({
        agent: structuredClone(requireRecord(state.agents, request.agentId, "agent")),
      }),
      create: async (request) => {
        const agent: Agent = {
          id: nextId("a", state.agents),
          name: request.name,
          description: request.description,
        };
        state.agents.push(agent);
        return { agent: structuredClone(agent) };
      },
      update: async (request) => {
        const index = requireRecordIndex(state.agents, request.agentId, "agent");
        const agent: Agent = {
          id: request.agentId,
          name: request.name,
          description: request.description,
        };
        state.agents[index] = agent;
        return { agent: structuredClone(agent) };
      },
      delete: async (request) => {
        removeRecord(state.agents, request.agentId);
        return { agentId: request.agentId };
      },
    },
    skill: {
      list: async () => ({ skills: structuredClone(state.skills) }),
      get: async (request) => ({
        skill: structuredClone(requireRecord(state.skills, request.skillId, "skill")),
      }),
      create: async (request) => {
        const skill: Skill = {
          id: nextId("sk", state.skills),
          name: request.name,
          description: request.description,
        };
        state.skills.push(skill);
        return { skill: structuredClone(skill) };
      },
      update: async (request) => {
        const index = requireRecordIndex(state.skills, request.skillId, "skill");
        const skill: Skill = {
          id: request.skillId,
          name: request.name,
          description: request.description,
        };
        state.skills[index] = skill;
        return { skill: structuredClone(skill) };
      },
      delete: async (request) => {
        removeRecord(state.skills, request.skillId);
        return { skillId: request.skillId };
      },
    },
    fileSystem: {
      listDirectory: async (request) => ({
        currentPath: request.path ?? "/workspace",
        parentPath: null,
        breadcrumbs: [],
        entries: [],
      }),
    },
    gitIdentity: {
      get: async () => ({ name: "Prototype User", email: "prototype@ora.local" }),
    },
  };
}

/** Returns one record or fails like the real not-found endpoint. */
function requireRecord<T extends { id: string }>(
  records: readonly T[],
  id: string,
  kind: string,
): T {
  const record = records.find((candidate) => candidate.id === id);
  if (record === undefined) {
    throw new Error(`${kind} ${id} not found`);
  }
  return record;
}

/** Returns a required record index for an in-place state update. */
function requireRecordIndex<T extends { id: string }>(
  records: readonly T[],
  id: string,
  kind: string,
): number {
  const index = records.findIndex((candidate) => candidate.id === id);
  if (index < 0) {
    throw new Error(`${kind} ${id} not found`);
  }
  return index;
}

/** Removes a record when present; deletes remain idempotent like the test backend. */
function removeRecord<T extends { id: string }>(records: T[], id: string): void {
  const index = records.findIndex((candidate) => candidate.id === id);
  if (index >= 0) {
    records.splice(index, 1);
  }
}
