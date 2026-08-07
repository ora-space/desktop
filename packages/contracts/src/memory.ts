import type * as acp from "./acp/index.js";
import type { Agent } from "./agent.js";
import type { AgentCli } from "./session.js";
import type { ContractsClient } from "./client.js";
import type { Project } from "./project.js";
import type { ProjectWorkContext } from "./project-work-context.js";
import type { Session } from "./session.js";
import type { Skill } from "./skill.js";
import type { Task, TaskStatus } from "./task.js";
import type { Workflow, WorkflowSnapshot, WorkflowSummary, WorkflowVersion } from "./workflow.js";

/** One in-memory workflow with its editable draft and published snapshot history. */
export interface MemoryWorkflowRecord {
  workflow: Workflow;
  draft: WorkflowSnapshot;
  published: WorkflowSnapshot[];
}

/** Mutable records owned by one in-memory ContractsClient instance. */
export interface MemoryContractsState {
  projects: Project[];
  tasks: Task[];
  sessions: Session[];
  agents: Array<Agent & { content?: string }>;
  skills: Array<Skill & { content?: string }>;
  workflows: MemoryWorkflowRecord[];
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
    workflows: structuredClone(seed.workflows ?? []),
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

/** Produces a millisecond-precision timestamp matching the contract's bigint wire type. */
function nextTimestamp(): bigint {
  return BigInt(Date.now());
}

/** Returns one workflow record or fails like the real not-found endpoint. */
function requireWorkflow(state: MemoryContractsState, workflowId: string): MemoryWorkflowRecord {
  const record = state.workflows.find((candidate) => candidate.workflow.id === workflowId);
  if (record === undefined) {
    throw new Error(`workflow ${workflowId} not found`);
  }
  return record;
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
        agent: {
          ...structuredClone(requireRecord(state.agents, request.agentId, "agent")),
          content: requireRecord(state.agents, request.agentId, "agent").content ?? "",
        },
      }),
      create: async (request) => {
        const agent: Agent & { content: string } = {
          id: nextId("a", state.agents),
          name: request.name,
          description: request.description,
          content: request.content ?? "",
        };
        state.agents.push(agent);
        return { agent: structuredClone(agent) };
      },
      update: async (request) => {
        const index = requireRecordIndex(state.agents, request.agentId, "agent");
        const agent: Agent & { content: string } = {
          id: request.agentId,
          name: request.name,
          description: request.description,
          content: request.content ?? state.agents[index]!.content ?? "",
        };
        state.agents[index] = agent;
        return { agent: structuredClone(agent) };
      },
      delete: async (request) => {
        removeRecord(state.agents, request.agentId);
        return { agentId: request.agentId };
      },
    },
    agentImport: {
      prepare: async () => {
        throw new Error("agentImport.prepare not implemented in memory client");
      },
      commit: async () => {
        throw new Error("agentImport.commit not implemented in memory client");
      },
    },
    skill: {
      list: async () => ({ skills: structuredClone(state.skills) }),
      get: async (request) => ({
        skill: {
          ...structuredClone(requireRecord(state.skills, request.skillId, "skill")),
          content: requireRecord(state.skills, request.skillId, "skill").content ?? "",
        },
      }),
      create: async (request) => {
        const skill: Skill & { content: string } = {
          id: nextId("sk", state.skills),
          name: request.name,
          description: request.description,
          content: request.content ?? "",
        };
        state.skills.push(skill);
        return { skill: structuredClone(skill) };
      },
      update: async (request) => {
        const index = requireRecordIndex(state.skills, request.skillId, "skill");
        const skill: Skill & { content: string } = {
          id: request.skillId,
          name: request.name,
          description: request.description,
          content: request.content ?? state.skills[index]!.content ?? "",
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
    spec: {
      catalog: async () => ({ sources: [], documents: [], truncated: false }),
      read: async () => {
        throw new Error("spec.read not implemented in memory client");
      },
      resolveSource: async () => {
        throw new Error("spec.resolveSource not implemented in memory client");
      },
      updateProjectSources: async (request) => ({
        sources: structuredClone(request.sources),
      }),
      watch: () =>
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
      create: async (request) => {
        const now = nextTimestamp();
        const id = nextId(
          "wf",
          state.workflows.map((record) => ({ id: record.workflow.id })),
        );
        const workflow: Workflow = {
          id,
          name: request.name,
          publishedSnapshotId: null,
          createdAt: now,
          updatedAt: now,
        };
        const draft: WorkflowSnapshot = {
          id: nextId("snap", []),
          workflowId: id,
          version: "draft",
          graph: request.graph ?? "{}",
          createdAt: now,
          updatedAt: now,
        };
        state.workflows.push({ workflow, draft, published: [] });
        return { workflow: structuredClone(workflow), draft: structuredClone(draft) };
      },
      get: async (request) => {
        const record = requireWorkflow(state, request.workflowId);
        const published = record.workflow.publishedSnapshotId == null
          ? null
          : record.published.find((item) => item.id === record.workflow.publishedSnapshotId) ?? null;
        return {
          workflow: structuredClone(record.workflow),
          draft: structuredClone(record.draft),
          published: published === null ? null : structuredClone(published),
        };
      },
      list: async () => ({
        workflows: state.workflows.map((record): WorkflowSummary => ({
          id: record.workflow.id,
          name: record.workflow.name,
          publishedVersion: record.workflow.publishedSnapshotId == null
            ? null
            : record.published.find((item) => item.id === record.workflow.publishedSnapshotId)?.version
              ?? null,
          createdAt: record.workflow.createdAt,
          updatedAt: record.workflow.updatedAt,
        })),
      }),
      update: async (request) => {
        const record = requireWorkflow(state, request.workflowId);
        record.workflow = {
          ...record.workflow,
          name: request.name,
          updatedAt: nextTimestamp(),
        };
        return { workflow: structuredClone(record.workflow) };
      },
      delete: async (request) => {
        const index = state.workflows.findIndex(
          (record) => record.workflow.id === request.workflowId,
        );
        if (index < 0) {
          throw new Error(`workflow ${request.workflowId} not found`);
        }
        state.workflows.splice(index, 1);
        return { workflowId: request.workflowId };
      },
      getDraft: async (request) => {
        const record = requireWorkflow(state, request.workflowId);
        return { snapshot: structuredClone(record.draft) };
      },
      updateDraft: async (request) => {
        const record = requireWorkflow(state, request.workflowId);
        record.draft = { ...record.draft, graph: request.graph, updatedAt: nextTimestamp() };
        return { snapshot: structuredClone(record.draft) };
      },
      publish: async (request) => {
        const record = requireWorkflow(state, request.workflowId);
        const now = nextTimestamp();
        const version = request.version ?? `v${now}`;
        const snapshot: WorkflowSnapshot = {
          id: nextId(
            "snap",
            record.published.map((item) => ({ id: item.id })),
          ),
          workflowId: record.workflow.id,
          version,
          graph: record.draft.graph,
          createdAt: now,
          updatedAt: null,
        };
        record.published.push(snapshot);
        record.workflow = { ...record.workflow, publishedSnapshotId: snapshot.id, updatedAt: now };
        return { snapshot: structuredClone(snapshot) };
      },
      listVersions: async (request) => {
        const record = requireWorkflow(state, request.workflowId);
        return {
          versions: record.published.map((snapshot): WorkflowVersion => ({
            id: snapshot.id,
            version: snapshot.version,
            createdAt: snapshot.createdAt,
          })),
        };
      },
      getVersion: async (request) => {
        const record = requireWorkflow(state, request.workflowId);
        const snapshot = record.published.find((item) => item.version === request.version);
        if (snapshot === undefined) {
          throw new Error(`workflow snapshot ${request.version} not found`);
        }
        return { snapshot: structuredClone(snapshot) };
      },
      rollback: async (request) => {
        const record = requireWorkflow(state, request.workflowId);
        const all = [...record.published, record.draft];
        const snapshot = all.find((item) => item.id === request.snapshotId);
        if (snapshot === undefined) {
          throw new Error(`snapshot ${request.snapshotId} not found`);
        }
        record.draft = { ...record.draft, graph: snapshot.graph, updatedAt: nextTimestamp() };
        return { snapshot: structuredClone(record.draft) };
      },
      activate: async (request) => {
        const record = requireWorkflow(state, request.workflowId);
        const snapshot = record.published.find((item) => item.id === request.snapshotId);
        if (snapshot === undefined) {
          throw new Error(`snapshot ${request.snapshotId} not found`);
        }
        record.workflow = {
          ...record.workflow,
          publishedSnapshotId: snapshot.id,
          updatedAt: nextTimestamp(),
        };
        record.draft = { ...record.draft, graph: snapshot.graph, updatedAt: nextTimestamp() };
        return { snapshot: structuredClone(record.draft) };
      },
      deleteSnapshot: async (request) => {
        const record = requireWorkflow(state, request.workflowId);
        const index = record.published.findIndex((item) => item.version === request.version);
        if (index < 0) {
          throw new Error(`snapshot ${request.version} not found`);
        }
        const [removed] = record.published.splice(index, 1);
        return { snapshotId: removed.id, version: request.version };
      },
      getSnapshot: async (request) => {
        for (const record of state.workflows) {
          const all = [...record.published, record.draft];
          const snapshot = all.find((item) => item.id === request.snapshotId);
          if (snapshot !== undefined) {
            return { snapshot: structuredClone(snapshot) };
          }
        }
        throw new Error(`snapshot ${request.snapshotId} not found`);
      },
    },
    workflowRun: {
      create: async () => {
        throw new Error("workflowRun.create not implemented in memory client");
      },
      get: async () => {
        throw new Error("workflowRun.get not implemented in memory client");
      },
      list: async () => {
        throw new Error("workflowRun.list not implemented in memory client");
      },
      listByWorkflow: async () => {
        throw new Error("workflowRun.listByWorkflow not implemented in memory client");
      },
      listNodeRuns: async () => {
        throw new Error("workflowRun.listNodeRuns not implemented in memory client");
      },
      delete: async () => {
        throw new Error("workflowRun.delete not implemented in memory client");
      },
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
