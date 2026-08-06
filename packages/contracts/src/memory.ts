import type * as acp from "./acp/index.js";
import type { Agent } from "./agent.js";
import type { AgentCli } from "./session.js";
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
  /** Warm sessions handed out but not yet attached, keyed by session id. */
  warmSessions: Map<string, AgentCli>;
  /** What every warm and persisted session reports as its configuration. */
  configOptions: acp.SessionConfigOption[];
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
    warmSessions: seed.warmSessions ?? new Map(),
    configOptions: structuredClone(seed.configOptions ?? [
      {
        id: "model",
        name: "Model",
        category: "model",
        type: "select",
        currentValue: "opencode/big-pickle",
        options: [
          { value: "opencode/big-pickle", name: "Big Pickle" },
          { value: "opencode/small-pickle", name: "Small Pickle" },
        ],
      },
    ]),
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
      listBranches: async () => ({
        branches: [{ name: "main", refName: "origin/main", displayName: "main" }],
      }),
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
          type: "default",
          workflowRunId: null,
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
      getWorkspace: async (request) => ({
        workspace: {
          rootPath: `/worktrees/${request.taskId}`,
          branchName: `task/${request.taskId}`,
        },
      }),
      getDiff: async () => ({
        baseCommitId: "base",
        headCommitId: "head",
        diffId: "diff",
        patch: "",
      }),
      commitChanges: async () => {
        throw new Error("commitChanges not implemented in memory client");
      },
      pushBranch: async () => {
        throw new Error("pushBranch not implemented in memory client");
      },
      listDiffComments: async () => ({ comments: [] }),
      createDiffComment: async () => {
        throw new Error("createDiffComment not implemented in memory client");
      },
      replyDiffComment: async () => {
        throw new Error("replyDiffComment not implemented in memory client");
      },
      setDiffCommentStatus: async () => {
        throw new Error("setDiffCommentStatus not implemented in memory client");
      },
    },
    session: {
      list: async () => ({ sessions: structuredClone(state.sessions) }),
      get: async (request) => ({
        session: structuredClone(requireRecord(state.sessions, request.sessionId, "session")),
      }),
      warm: async (request) => {
        const sessionId = nextId("s", [
          ...state.sessions,
          ...[...state.warmSessions.keys()].map((id) => ({ id })),
        ]);
        state.warmSessions.set(sessionId, request.agentCli);
        return {
          sessionId,
          configOptions: structuredClone(state.configOptions),
        };
      },
      setConfig: async () => ({
        configOptions: structuredClone(state.configOptions),
      }),
      attach: async (request) => {
        const session: Session = {
          id: request.sessionId,
          taskId: request.taskId,
          agentCli: state.warmSessions.get(request.sessionId) ?? "open_code",
          status: "running",
          historyState: { type: "writable" },
        };
        state.warmSessions.delete(request.sessionId);
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
      switchAgent: async (request) => {
        const index = requireRecordIndex(state.sessions, request.sessionId, "session");
        const session: Session = {
          ...state.sessions[index]!,
          agentCli: request.agentCli,
        };
        state.sessions[index] = session;
        return {
          session: structuredClone(session),
          availableCommands: [],
          configOptions: structuredClone(state.configOptions),
        };
      },
      resumeHistory: async (request) => {
        const index = requireRecordIndex(state.sessions, request.sessionId, "session");
        const session: Session = {
          ...state.sessions[index]!,
          historyState: { type: "writable" },
        };
        state.sessions[index] = session;
        return { session: structuredClone(session) };
      },
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
    skillImport: {
      prepare: async () => {
        throw new Error("skillImport.prepare not implemented in memory client");
      },
      get: async () => {
        throw new Error("skillImport.get not implemented in memory client");
      },
      commit: async () => {
        throw new Error("skillImport.commit not implemented in memory client");
      },
      cancel: async (request) => ({
        sessionId: request.sessionId,
        cancelled: true,
      }),
    },
    fileSystem: {
      listDirectory: async (request) => ({
        currentPath: request.path ?? "/workspace",
        parentPath: null,
        breadcrumbs: [],
        entries: [],
      }),
      listWorkspaceDirectory: async () => ({ path: "", entries: [] }),
      readWorkspaceFile: async (request) => ({
        path: request.path,
        content: "",
        version: "test",
        sizeBytes: 0,
      }),
      searchWorkspace: async () => ({ results: [], truncated: false }),
      watchWorkspace: () =>
        (async function* () {
          yield* [];
        })(),
    },
    gitIdentity: {
      get: async () => ({ name: "Prototype User", email: "prototype@ora.local" }),
    },
    agentRuntime: {
      getStatus: async () => ({
        statuses: [
          { agentCli: "open_code", status: "ready" },
          { agentCli: "nga", status: "ready" },
          { agentCli: "code_agent_cli", status: "ready" },
          { agentCli: "claude", status: "ready" },
          { agentCli: "codex", status: "ready" },
        ],
      }),
    },
    workflow: {
      create: async () => { throw new Error("workflow not implemented in memory client"); },
      get: async () => { throw new Error("workflow not implemented in memory client"); },
      list: async () => { throw new Error("workflow not implemented in memory client"); },
      update: async () => { throw new Error("workflow not implemented in memory client"); },
      delete: async () => { throw new Error("workflow not implemented in memory client"); },
      getDraft: async () => { throw new Error("workflow not implemented in memory client"); },
      updateDraft: async () => { throw new Error("workflow not implemented in memory client"); },
      publish: async () => { throw new Error("workflow not implemented in memory client"); },
      rollback: async () => { throw new Error("workflow not implemented in memory client"); },
      activate: async () => { throw new Error("workflow not implemented in memory client"); },
      listVersions: async () => { throw new Error("workflow not implemented in memory client"); },
      getVersion: async () => { throw new Error("workflow not implemented in memory client"); },
      deleteSnapshot: async () => { throw new Error("workflow not implemented in memory client"); },
    },
    workflowRun: {
      create: async () => { throw new Error("workflowRun not implemented in memory client"); },
      get: async () => { throw new Error("workflowRun not implemented in memory client"); },
      list: async () => { throw new Error("workflowRun not implemented in memory client"); },
      listNodeRuns: async () => { throw new Error("workflowRun not implemented in memory client"); },
      delete: async () => { throw new Error("workflowRun not implemented in memory client"); },
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
