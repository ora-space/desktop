import type {
  acp,
  Agent,
  AgentCli,
  ContractsClient,
  Project,
  Session,
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
  /** Warm sessions handed out but not yet attached, keyed by session id. */
  warmSessions: Map<string, AgentCli>;
  /** What every warm and persisted session reports as its configuration. */
  configOptions: acp.SessionConfigOption[];
}

/** Creates a fresh in-memory mock state with no records. */
export function createMockClientState(): MockClientState {
  return {
    projects: [],
    tasks: [],
    sessions: [],
    agents: [],
    skills: [],
    warmSessions: new Map(),
    configOptions: [
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
    ],
  };
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
      listBranches: async () => ({
        branches: [{ name: "main", refName: "origin/main", displayName: "main" }],
      }),
      get: async (req) => ({ project: state.projects.find((p) => p.id === req.projectId)! }),
      create: async (req) => {
        const project: Project = { id: nextId("p", state.projects.length), name: req.name, rootPath: req.rootPath };
        state.projects.push(project);
        return { project };
      },
      update: async (req) => {
        const idx = state.projects.findIndex((p) => p.id === req.projectId);
        if (idx < 0) throw new Error(`project ${req.projectId} not found`);
        const updated: Project = { ...state.projects[idx]!, name: req.name };
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
          workspaceMode: req.workspaceMode ?? "worktree",
          type: "default",
          workflowRunId: null,
        };
        state.tasks.push(task);
        return { task };
      },
      update: async (req) => {
        const idx = state.tasks.findIndex((t) => t.id === req.taskId);
        if (idx < 0) throw new Error(`task ${req.taskId} not found`);
        const updated: Task = {
          ...state.tasks[idx]!,
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
      getWorkspace: async (req) => ({
        workspace: {
          rootPath: `/worktrees/${req.taskId}`,
          branchName: `task/${req.taskId}`,
        },
      }),
      getDiff: async () => ({
        baseCommitId: "base",
        headCommitId: "head",
        diffId: "diff",
        patch: "",
      }),
      commitChanges: async () => {
        throw new Error("commitChanges not implemented in mock");
      },
      pushBranch: async () => {
        throw new Error("pushBranch not implemented in mock");
      },
      listDiffComments: async () => ({ comments: [] }),
      createDiffComment: async () => {
        throw new Error("createDiffComment not implemented in mock");
      },
      replyDiffComment: async () => {
        throw new Error("replyDiffComment not implemented in mock");
      },
      setDiffCommentStatus: async () => {
        throw new Error("setDiffCommentStatus not implemented in mock");
      },
    },
    session: {
      list: async () => ({ sessions: [...state.sessions] }),
      get: async (req) => ({ session: state.sessions.find((s) => s.id === req.sessionId)! }),
      warm: async (req) => {
        const sessionId = nextId("s", state.sessions.length + state.warmSessions.size);
        state.warmSessions.set(sessionId, req.agentCli);
        return { sessionId, configOptions: state.configOptions };
      },
      setConfig: async () => ({ configOptions: state.configOptions }),
      attach: async (req) => {
        const session: Session = {
          id: req.sessionId,
          taskId: req.taskId,
          agentCli: state.warmSessions.get(req.sessionId) ?? "open_code",
          status: "running",
          historyState: { type: "writable" },
        };
        state.warmSessions.delete(req.sessionId);
        state.sessions.push(session);
        return { session, availableCommands: [] };
      },
      switchAgent: async (req) => {
        const session = state.sessions.find((candidate) => candidate.id === req.sessionId)!;
        session.agentCli = req.agentCli;
        return { session, availableCommands: [], configOptions: state.configOptions };
      },
      resumeHistory: async (req) => {
        const session = state.sessions.find((candidate) => candidate.id === req.sessionId)!;
        session.historyState = { type: "writable" };
        return { session };
      },
      load: async function* () { yield { type: "completed" as const }; },
      prompt: async function* () { yield { type: "completed" as const, stopReason: "end_turn" as const }; },
      respondToPermission: async () => ({}),
      switchAgent: async (req) => {
        const session = state.sessions.find((candidate) => candidate.id === req.sessionId)!;
        session.agentCli = req.agentCli;
        return { session, availableCommands: [] };
      },
      resumeHistory: async (req) => {
        const session = state.sessions.find((candidate) => candidate.id === req.sessionId)!;
        session.historyState = { type: "writable" };
        return { session };
      },
      stop: async (req) => {
        const session = state.sessions.find((candidate) => candidate.id === req.sessionId)!;
        session.status = "stopped";
        return { session };
      },
      delete: async (req) => {
        const idx = state.sessions.findIndex((s) => s.id === req.sessionId);
        if (idx >= 0) state.sessions.splice(idx, 1);
        return { sessionId: req.sessionId };
      },
    },
    agentRuntime: {
      getStatus: async () => ({
        statuses: [
          { agentCli: "open_code", status: "ready" },
          { agentCli: "nga", status: "ready" },
          { agentCli: "code_agent_cli", status: "ready" },
        ],
      }),
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
    skillImport: {
      prepare: async () => { throw new Error("skillImport not implemented in mock"); },
      get: async () => { throw new Error("skillImport not implemented in mock"); },
      commit: async () => { throw new Error("skillImport not implemented in mock"); },
      cancel: async () => { throw new Error("skillImport not implemented in mock"); },
    },
    fileSystem: {
      listDirectory: async (request) => ({
        currentPath: request.path ?? "/home/test",
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
      watchWorkspace: () => (async function* () {
        yield* [];
      })(),
    },
    spec: {
      catalog: async () => ({ sources: [], documents: [], truncated: false }),
      read: async (request) => ({
        relativePath: request.relativePath,
        content: "",
        byteSize: 0,
      }),
      resolveSource: async () => ({
        relativePath: "docs/specs",
        workflow: { kind: "custom", name: "Custom" },
      }),
      updateProjectSources: async (request) => ({ sources: request.sources }),
      watch: () => (async function* () {
        yield* [];
      })(),
    },
    gitIdentity: {
      get: async () => { throw new Error("gitIdentity not implemented in mock"); },
    },
    workflow: {
      create: async () => { throw new Error("workflow not implemented in mock"); },
      get: async () => { throw new Error("workflow not implemented in mock"); },
      list: async () => { throw new Error("workflow not implemented in mock"); },
      update: async () => { throw new Error("workflow not implemented in mock"); },
      delete: async () => { throw new Error("workflow not implemented in mock"); },
      getDraft: async () => { throw new Error("workflow not implemented in mock"); },
      updateDraft: async () => { throw new Error("workflow not implemented in mock"); },
      publish: async () => { throw new Error("workflow not implemented in mock"); },
      rollback: async () => { throw new Error("workflow not implemented in mock"); },
      activate: async () => { throw new Error("workflow not implemented in mock"); },
      listVersions: async () => { throw new Error("workflow not implemented in mock"); },
      getVersion: async () => { throw new Error("workflow not implemented in mock"); },
      deleteSnapshot: async () => { throw new Error("workflow not implemented in mock"); },
    },
    workflowRun: {
      create: async () => { throw new Error("workflowRun not implemented in mock"); },
      get: async () => { throw new Error("workflowRun not implemented in mock"); },
      list: async () => { throw new Error("workflowRun not implemented in mock"); },
      listNodeRuns: async () => { throw new Error("workflowRun not implemented in mock"); },
      delete: async () => { throw new Error("workflowRun not implemented in mock"); },
    },
  };
}
