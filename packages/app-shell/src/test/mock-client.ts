import type * as acp from "@agentclientprotocol/sdk";
import type {
  Agent,
  AgentRuntimeStatus,
  AvailablePlugin,
  ContractsClient,
  InstalledPlugin,
  Project,
  RuntimeLogLevelStateResponse,
  Session,
  Skill,
  Task,
  Workflow,
  WorkflowRun,
  WorkflowSnapshot,
  WorkflowSummary,
  WorkflowVersion,
} from "@ora/contracts";

/** One in-memory workflow with its editable draft and published history. */
export interface MockWorkflowRecord {
  workflow: Workflow;
  draft: WorkflowSnapshot;
  published: WorkflowSnapshot[];
}

/** One in-memory workflow run used by run-hook tests. */
export interface MockWorkflowRunRecord {
  id: string;
  projectId: string;
  workflowId: string;
  snapshotId: string;
  name: string;
  status: "pending" | "running" | "succeeded" | "failed" | "cancelled";
  taskId: string;
  createdAt: bigint;
  updatedAt: bigint;
}

/** Builds the public run payload for one mock workflow-run record. */
function mockWorkflowRun(record: MockWorkflowRunRecord): WorkflowRun {
  return {
    id: record.id,
    workflowId: record.workflowId,
    snapshotId: record.snapshotId,
    status: record.status,
    state: '{"current_nodes":[]}',
    input: null,
    output: null,
    error: null,
    payload: null,
    startedAt: null,
    finishedAt: null,
    createdAt: record.createdAt,
    updatedAt: record.updatedAt,
  };
}

/** In-memory state mutated by the mock client so tests can assert post-call state. */
export interface MockClientState {
  projects: Project[];
  tasks: Task[];
  sessions: Session[];
  agents: Agent[];
  skills: Skill[];
  installedPlugins: InstalledPlugin[];
  /**
   * What the agent runtime reports reaching, which is what decides the agents the pickers offer.
   *
   * An agent missing from this list is one nothing supervises — an uninstalled plugin package.
   */
  agentRuntimeStatuses: AgentRuntimeStatus[];
  availablePlugins: AvailablePlugin[];
  availablePluginsUpdatedAt: bigint;
  /**
   * The package a local `.orax` import should materialize; `null` rejects that import.
   * Undefined means imports are not configured and always fail in tests. A concrete target is
   * committed with `enabled: true`, mirroring the backend finalize step that auto-enables the
   * imported package.
   */
  importTarget?: InstalledPlugin | null;
  developerMode: { enabled: boolean };
  runtimeLogLevel: RuntimeLogLevelStateResponse;
  workflows: MockWorkflowRecord[];
  workflowRuns: MockWorkflowRunRecord[];
  /** Warm sessions handed out but not yet attached, keyed by session id. */
  warmSessions: Map<string, string>;
  /** What every warm and persisted session reports as its configuration. */
  configOptions: acp.SessionConfigOption[];
  /**
   * Per-CLI warm-session config overrides for tests. A CLI mapped to `null`
   * reports no model catalog (warm failed); a CLI mapped to an array uses
   * those options instead of the shared `configOptions`.
   */
  warmModelsByCli?: Partial<Record<string, acp.SessionConfigOption[] | null>>;
}

/**
 * Every agent identity the frontend has a picker entry for, all detected by default.
 *
 * A test that needs one to be missing or unreachable overrides `agentRuntimeStatuses` rather than
 * rebuilding the whole list.
 */
const AGENT_REFS = [
  "ora-space.opencode",
  "ora-space.nga",
  "ora-space.codeagentcli",
  "ora-space.claude",
  "ora-space.codex",
];

/** Creates a fresh in-memory mock state with no records. */
export function createMockClientState(): MockClientState {
  return {
    projects: [],
    tasks: [],
    sessions: [],
    agents: [],
    skills: [],
    installedPlugins: [],
    agentRuntimeStatuses: AGENT_REFS.map((agentRef) => ({
      agentRef,
      status: "ready",
    })),
    availablePlugins: [],
    availablePluginsUpdatedAt: 0n,
    developerMode: { enabled: false },
    runtimeLogLevel: {
      configuredLevel: "info",
      effectiveLevel: "info",
      startupOverride: null,
    },
    workflows: [],
    workflowRuns: [],
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

/** Produces a millisecond-precision timestamp matching the contract's bigint wire type. */
function nextTimestamp(): bigint {
  return BigInt(Date.now());
}

/** Returns one workflow record or fails like the real not-found endpoint. */
function requireWorkflowRecord(
  state: MockClientState,
  workflowId: string,
): MockWorkflowRecord {
  const record = state.workflows.find(
    (candidate) => candidate.workflow.id === workflowId,
  );
  if (record === undefined) {
    throw new Error(`workflow ${workflowId} not found`);
  }
  return record;
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
        branches: [
          { name: "main", refName: "origin/main", displayName: "main" },
        ],
      }),
      get: async (req) => ({
        project: state.projects.find((p) => p.id === req.projectId)!,
      }),
      create: async (req) => {
        const project: Project = {
          id: nextId("p", state.projects.length),
          name: req.name,
          rootPath: req.rootPath,
        };
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
    task: {
      list: async () => ({ tasks: [...state.tasks] }),
      get: async (req) => ({
        task: state.tasks.find((t) => t.id === req.taskId)!,
      }),
      create: async (req) => {
        const task: Task = {
          id: nextId("t", state.tasks.length),
          projectId: req.projectId,
          title: req.title,
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
        };
        state.tasks[idx] = updated;
        // Production lists derive the run display name from the run-task title.
        for (const run of state.workflowRuns) {
          if (run.taskId === req.taskId) run.name = req.title;
        }
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
        patch: "",
      }),
      commitChanges: async () => {
        throw new Error("commitChanges not implemented in mock");
      },
      pushBranch: async () => {
        throw new Error("pushBranch not implemented in mock");
      },
    },
    session: {
      list: async () => ({ sessions: [...state.sessions] }),
      get: async (req) => ({
        session: state.sessions.find((s) => s.id === req.sessionId)!,
      }),
      warm: async (req) => {
        const sessionId = nextId(
          "s",
          state.sessions.length + state.warmSessions.size,
        );
        state.warmSessions.set(sessionId, req.agentRef);
        const perCli = state.warmModelsByCli?.[req.agentRef];
        return {
          sessionId,
          // A CLI mapped to null reports an empty catalog, which is how the
          // contract expresses "no models" after a failed warm handshake.
          configOptions:
            perCli === undefined ? state.configOptions : (perCli ?? []),
        };
      },
      setConfig: async () => ({ configOptions: state.configOptions }),
      attach: async (req) => {
        const session: Session = {
          id: req.sessionId,
          taskId: req.taskId,
          agentRef:
            state.warmSessions.get(req.sessionId) ?? "ora-space.opencode",
          status: "running",
          title: null,
          historyState: { type: "writable" },
        };
        state.warmSessions.delete(req.sessionId);
        state.sessions.push(session);
        return { session, availableCommands: [] };
      },
      switchAgent: async (req) => {
        const session = state.sessions.find(
          (candidate) => candidate.id === req.sessionId,
        )!;
        session.agentRef = req.agentRef;
        return {
          session,
          availableCommands: [],
          configOptions: state.configOptions,
        };
      },
      resumeHistory: async (req) => {
        const session = state.sessions.find(
          (candidate) => candidate.id === req.sessionId,
        )!;
        session.historyState = { type: "writable" };
        return { session };
      },
      load: async function* () {
        yield { type: "completed" as const };
      },
      prompt: async function* () {
        yield { type: "completed" as const, stopReason: "end_turn" as const };
      },
      respondToPermission: async () => ({}),
      cancelPrompt: async () => ({}),
      stop: async (req) => {
        const session = state.sessions.find(
          (candidate) => candidate.id === req.sessionId,
        )!;
        session.status = "stopped";
        return { session };
      },
      delete: async (req) => {
        const idx = state.sessions.findIndex((s) => s.id === req.sessionId);
        if (idx >= 0) state.sessions.splice(idx, 1);
        return { sessionId: req.sessionId };
      },
      rename: async (req) => {
        const idx = state.sessions.findIndex((s) => s.id === req.sessionId);
        const current = state.sessions[idx]!;
        const session = { ...current, title: req.title };
        state.sessions[idx] = session;
        return { session };
      },
    },
    appEvents: {
      watch: async function* (_request, options) {
        yield { type: "ready" as const };
        await new Promise<void>((resolve) => {
          const signal = options?.signal;
          if (signal === undefined) return;
          if (signal.aborted) {
            resolve();
            return;
          }
          signal.addEventListener("abort", () => resolve(), { once: true });
        });
      },
    },
    agentRuntime: {
      getStatus: async () => ({ statuses: [...state.agentRuntimeStatuses] }),
    },
    plugin: {
      listInstalled: async () => ({ plugins: [...state.installedPlugins] }),
      listAvailable: async () => ({
        updatedAt: state.availablePluginsUpdatedAt,
        plugins: [...state.availablePlugins],
      }),
      syncAvailable: async () => ({
        updatedAt: state.availablePluginsUpdatedAt,
        plugins: [...state.availablePlugins],
      }),
      scan: async () => ({ plugins: [...state.installedPlugins] }),
      enable: async (req) => {
        const plugin = state.installedPlugins.find(
          (p) => p.id === req.pluginId,
        );
        if (!plugin)
          throw new Error(`installed plugin ${req.pluginId} not found`);
        plugin.enabled = true;
        // Enabling a plugin is also what starts it, so the backend answers with the
        // starting runtime and reports running once the process is up.
        plugin.runtime = "starting";
        return { plugin };
      },
      disable: async (req) => {
        const plugin = state.installedPlugins.find(
          (p) => p.id === req.pluginId,
        );
        if (!plugin)
          throw new Error(`installed plugin ${req.pluginId} not found`);
        plugin.enabled = false;
        plugin.runtime = "stopped";
        return { plugin };
      },
      activate: async (req) => {
        const plugin = state.installedPlugins.find(
          (p) => p.id === req.pluginId,
        );
        if (!plugin)
          throw new Error(`installed plugin ${req.pluginId} not found`);
        plugin.runtime = "running";
        return { plugin };
      },
      stop: async (req) => {
        const plugin = state.installedPlugins.find(
          (p) => p.id === req.pluginId,
        );
        if (!plugin)
          throw new Error(`installed plugin ${req.pluginId} not found`);
        plugin.runtime = "stopped";
        return { plugin };
      },
      uninstall: async (req) => {
        const idx = state.installedPlugins.findIndex(
          (p) => p.id === req.pluginId,
        );
        if (idx < 0)
          throw new Error(`installed plugin ${req.pluginId} not found`);
        state.installedPlugins.splice(idx, 1);
        return { pluginId: req.pluginId };
      },
      import: async (req) => {
        const target = state.importTarget;
        if (target === undefined)
          throw new Error(`import not configured for ${req.path}`);
        if (target === null) throw new Error(`import failed for ${req.path}`);
        state.installedPlugins.push({ ...target, enabled: true });
        return { pluginId: target.id };
      },
      install: async (req) => {
        const available = state.availablePlugins.find(
          (p) => p.id === req.pluginId,
        );
        if (!available)
          throw new Error(`available plugin ${req.pluginId} not found`);
        state.installedPlugins.push({
          id: available.id,
          namespace: available.namespace,
          name: available.name,
          displayName: available.name,
          version: available.version,
          description: available.description,
          homepage: null,
          license: null,
          kind: "agent",
          agentDisplayName: available.name,
          enabled: true,
          logo: available.logo,
          runtime: "stopped",
        });
        return { pluginId: req.pluginId };
      },
    },
    agent: {
      list: async () => ({ agents: [...state.agents] }),
      get: async (req) => ({
        agent: {
          ...state.agents.find((a) => a.id === req.agentId)!,
          content: "",
        },
      }),
      create: async (req) => {
        const agent: Agent = {
          id: nextId("a", state.agents.length),
          namespace: "local",
          name: req.name,
          description: req.description,
        };
        state.agents.push(agent);
        return { agent };
      },
      update: async (req) => {
        const idx = state.agents.findIndex((a) => a.id === req.agentId);
        if (idx < 0) throw new Error(`agent ${req.agentId} not found`);
        const updated: Agent = {
          id: req.agentId,
          namespace: state.agents[idx].namespace,
          name: req.name,
          description: req.description,
        };
        state.agents[idx] = updated;
        return { agent: updated };
      },
      delete: async (req) => {
        const idx = state.agents.findIndex((a) => a.id === req.agentId);
        if (idx >= 0) state.agents.splice(idx, 1);
        return { agentId: req.agentId };
      },
    },
    agentImport: {
      prepare: async () => {
        throw new Error("agentImport not implemented in mock");
      },
      commit: async () => {
        throw new Error("agentImport not implemented in mock");
      },
    },
    skill: {
      list: async () => ({ skills: [...state.skills] }),
      get: async (req) => ({
        skill: {
          ...state.skills.find((s) => s.id === req.skillId)!,
          content: "",
        },
      }),
      create: async (req) => {
        const skill: Skill = {
          id: nextId("sk", state.skills.length),
          namespace: "local",
          name: req.name,
          description: req.description,
          availability: "available",
        };
        state.skills.push(skill);
        return { skill };
      },
      update: async (req) => {
        const idx = state.skills.findIndex((s) => s.id === req.skillId);
        if (idx < 0) throw new Error(`skill ${req.skillId} not found`);
        const existing = state.skills[idx]!;
        const updated: Skill = {
          id: req.skillId,
          namespace: existing.namespace,
          name: req.name,
          description: req.description,
          availability: existing.availability,
        };
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
      prepare: async () => {
        throw new Error("skillImport.prepare not implemented in mock");
      },
      get: async () => {
        throw new Error("skillImport.get not implemented in mock");
      },
      commit: async () => {
        throw new Error("skillImport.commit not implemented in mock");
      },
      cancel: async (request) => ({
        sessionId: request.sessionId,
        cancelled: true,
      }),
    },
    fileSystem: {
      listWorkspaceDirectory: async () => ({ path: "", entries: [] }),
      listProjectDirectory: async () => ({ path: "", entries: [] }),
      readProjectFile: async (request) => ({
        path: request.path,
        content: "",
        version: "test",
        sizeBytes: 0,
      }),
      readWorkspaceFile: async (request) => ({
        path: request.path,
        content: "",
        version: "test",
        sizeBytes: 0,
      }),
      searchWorkspace: async () => ({ results: [], truncated: false }),
      searchProject: async () => ({ results: [], truncated: false }),
      watchWorkspace: () =>
        (async function* () {
          yield* [];
        })(),
      watchProject: () =>
        (async function* () {
          yield* [];
        })(),
    },
    spec: {
      catalog: async () => ({ documents: [], truncated: false }),
      read: async (request) => ({
        relativePath: request.relativePath,
        content: "",
        byteSize: 0,
      }),
      watch: () =>
        (async function* () {
          yield* [];
        })(),
    },
    gitIdentity: {
      get: async () => ({ name: "Test User", email: "test@ora.local" }),
    },
    developerMode: {
      get: async () => ({ ...state.developerMode }),
      set: async (request) => {
        state.developerMode = { enabled: request.enabled };
        return { ...state.developerMode };
      },
    },
    runtimeLogLevel: {
      get: async () => ({ ...state.runtimeLogLevel }),
      set: async (request) => {
        state.runtimeLogLevel = {
          configuredLevel: request.level,
          effectiveLevel: request.level,
          startupOverride: state.runtimeLogLevel.startupOverride,
        };
        return { ...state.runtimeLogLevel };
      },
    },
    workflow: {
      create: async (req) => {
        const now = nextTimestamp();
        const id = nextId("wf", state.workflows.length);
        const workflow: Workflow = {
          id,
          namespace: "local",
          name: req.name,
          publishedSnapshotId: null,
          createdAt: now,
          updatedAt: now,
        };
        const draft: WorkflowSnapshot = {
          id: nextId("snap", 0),
          workflowId: id,
          version: "draft",
          graph: req.graph ?? "{}",
          createdAt: now,
          updatedAt: now,
        };
        state.workflows.push({ workflow, draft, published: [] });
        return { workflow, draft };
      },
      get: async (req) => {
        const record = requireWorkflowRecord(state, req.workflowId);
        const published =
          record.workflow.publishedSnapshotId == null
            ? null
            : (record.published.find(
                (item) => item.id === record.workflow.publishedSnapshotId,
              ) ?? null);
        return {
          workflow: record.workflow,
          draft: record.draft,
          published,
        };
      },
      list: async () => ({
        workflows: state.workflows.map((record): WorkflowSummary => ({
          id: record.workflow.id,
          namespace: record.workflow.namespace,
          name: record.workflow.name,
          publishedVersion:
            record.workflow.publishedSnapshotId == null
              ? null
              : (record.published.find(
                  (item) => item.id === record.workflow.publishedSnapshotId,
                )?.version ?? null),
          createdAt: record.workflow.createdAt,
          updatedAt: record.workflow.updatedAt,
        })),
      }),
      update: async (req) => {
        const record = requireWorkflowRecord(state, req.workflowId);
        record.workflow = {
          ...record.workflow,
          name: req.name,
          updatedAt: nextTimestamp(),
        };
        return { workflow: record.workflow };
      },
      delete: async (req) => {
        const idx = state.workflows.findIndex(
          (record) => record.workflow.id === req.workflowId,
        );
        if (idx >= 0) state.workflows.splice(idx, 1);
        return { workflowId: req.workflowId };
      },
      getDraft: async (req) => {
        const record = requireWorkflowRecord(state, req.workflowId);
        return { snapshot: record.draft };
      },
      updateDraft: async (req) => {
        const record = requireWorkflowRecord(state, req.workflowId);
        record.draft = {
          ...record.draft,
          graph: req.graph,
          updatedAt: nextTimestamp(),
        };
        return { snapshot: record.draft };
      },
      publish: async (req) => {
        const record = requireWorkflowRecord(state, req.workflowId);
        const now = nextTimestamp();
        const version = req.version ?? `v${now}`;
        const snapshot: WorkflowSnapshot = {
          id: nextId("snap", record.published.length),
          workflowId: record.workflow.id,
          version,
          graph: record.draft.graph,
          createdAt: now,
          updatedAt: null,
        };
        record.published.push(snapshot);
        record.workflow = {
          ...record.workflow,
          publishedSnapshotId: snapshot.id,
          updatedAt: now,
        };
        return { snapshot };
      },
      rollback: async (req) => {
        const record = requireWorkflowRecord(state, req.workflowId);
        const all = [...record.published, record.draft];
        const snapshot = all.find((item) => item.id === req.snapshotId);
        if (snapshot === undefined)
          throw new Error(`snapshot ${req.snapshotId} not found`);
        record.draft = {
          ...record.draft,
          graph: snapshot.graph,
          updatedAt: nextTimestamp(),
        };
        return { snapshot: record.draft };
      },
      activate: async (req) => {
        const record = requireWorkflowRecord(state, req.workflowId);
        const snapshot = record.published.find(
          (item) => item.id === req.snapshotId,
        );
        if (snapshot === undefined)
          throw new Error(`snapshot ${req.snapshotId} not found`);
        record.workflow = {
          ...record.workflow,
          publishedSnapshotId: snapshot.id,
          updatedAt: nextTimestamp(),
        };
        record.draft = {
          ...record.draft,
          graph: snapshot.graph,
          updatedAt: nextTimestamp(),
        };
        return { snapshot: record.draft };
      },
      listVersions: async (req) => {
        const record = requireWorkflowRecord(state, req.workflowId);
        return {
          versions: record.published.map((snapshot): WorkflowVersion => ({
            id: snapshot.id,
            version: snapshot.version,
            createdAt: snapshot.createdAt,
          })),
        };
      },
      getVersion: async (req) => {
        const record = requireWorkflowRecord(state, req.workflowId);
        const snapshot = record.published.find(
          (item) => item.version === req.version,
        );
        if (snapshot === undefined)
          throw new Error(`snapshot ${req.version} not found`);
        return { snapshot };
      },
      deleteSnapshot: async (req) => {
        const record = requireWorkflowRecord(state, req.workflowId);
        const idx = record.published.findIndex(
          (item) => item.version === req.version,
        );
        if (idx < 0) throw new Error(`snapshot ${req.version} not found`);
        const [removed] = record.published.splice(idx, 1);
        return { snapshotId: removed.id, version: req.version };
      },
      getSnapshot: async (req) => {
        for (const record of state.workflows) {
          const all = [...record.published, record.draft];
          const snapshot = all.find((item) => item.id === req.snapshotId);
          if (snapshot !== undefined) return { snapshot };
        }
        throw new Error(`snapshot ${req.snapshotId} not found`);
      },
    },
    workflowRun: {
      create: async (req) => {
        const id = nextId("run", state.workflowRuns.length);
        const now = nextTimestamp();
        const run: WorkflowRun = {
          id,
          workflowId: req.workflowId,
          snapshotId: "snap-1",
          status: "pending",
          state: '{"current_nodes":[]}',
          input: null,
          output: null,
          error: null,
          payload: null,
          startedAt: null,
          finishedAt: null,
          createdAt: now,
          updatedAt: now,
        };
        state.workflowRuns.push({
          id,
          projectId: req.projectId,
          workflowId: req.workflowId,
          snapshotId: run.snapshotId,
          name: req.name ?? "",
          status: "pending",
          taskId: nextId("task", state.tasks.length),
          createdAt: now,
          updatedAt: now,
        });
        return { run, taskId: nextId("task", state.tasks.length) };
      },
      get: async (req) => {
        const record = state.workflowRuns.find(
          (candidate) => candidate.id === req.runId,
        );
        if (record === undefined)
          throw new Error(`workflow run ${req.runId} not found`);
        return {
          run: {
            id: record.id,
            workflowId: record.workflowId,
            snapshotId: record.snapshotId,
            status: record.status,
            state: '{"current_nodes":[]}',
            input: null,
            output: null,
            error: null,
            payload: null,
            startedAt: null,
            finishedAt: null,
            createdAt: record.createdAt,
            updatedAt: record.updatedAt,
          },
          name: record.name,
          projectId: record.projectId,
          taskId: record.taskId,
          nodes: [],
        };
      },
      start: async (req) => {
        const record = state.workflowRuns.find(
          (candidate) => candidate.id === req.runId,
        );
        if (record === undefined)
          throw new Error(`workflow run ${req.runId} not found`);
        return { run: mockWorkflowRun(record) };
      },
      cancel: async (req) => {
        const record = state.workflowRuns.find(
          (candidate) => candidate.id === req.runId,
        );
        if (record === undefined)
          throw new Error(`workflow run ${req.runId} not found`);
        return { run: mockWorkflowRun(record) };
      },
      restart: async (req) => {
        const record = state.workflowRuns.find(
          (candidate) => candidate.id === req.runId,
        );
        if (record === undefined)
          throw new Error(`workflow run ${req.runId} not found`);
        return { run: mockWorkflowRun(record) };
      },
      updateInput: async (req) => {
        const record = state.workflowRuns.find(
          (candidate) => candidate.id === req.runId,
        );
        if (record === undefined)
          throw new Error(`workflow run ${req.runId} not found`);
        return { run: mockWorkflowRun(record) };
      },
      completeNode: async (req) => {
        const record = state.workflowRuns.find(
          (candidate) => candidate.id === req.runId,
        );
        if (record === undefined)
          throw new Error(`workflow run ${req.runId} not found`);
        record.status = "succeeded";
        return { run: mockWorkflowRun(record) };
      },
      list: async (req) => ({
        runs: state.workflowRuns
          .filter((record) => record.projectId === req.projectId)
          .map((record) => ({
            id: record.id,
            name: record.name,
            projectId: record.projectId,
            workflowId: record.workflowId,
            status: record.status,
            startedAt: null,
            finishedAt: null,
            createdAt: record.createdAt,
          })),
      }),
      listByWorkflow: async (req) => ({
        runs: state.workflowRuns
          .filter((record) => record.workflowId === req.workflowId)
          .map((record) => ({
            id: record.id,
            name: record.name,
            projectId: record.projectId,
            workflowId: record.workflowId,
            status: record.status,
            startedAt: null,
            finishedAt: null,
            createdAt: record.createdAt,
          })),
      }),
      listNodeRuns: async () => ({ nodes: [] }),
      delete: async (req) => {
        const idx = state.workflowRuns.findIndex(
          (record) => record.id === req.runId,
        );
        if (idx >= 0) state.workflowRuns.splice(idx, 1);
        return { runId: req.runId };
      },
    },
  };
}
