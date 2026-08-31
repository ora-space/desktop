import type * as acp from "@agentclientprotocol/sdk";
import {
  RemoteContractError,
  type Agent,
  type AgentRuntimeStatus,
  type AvailablePlugin,
  type MarketplaceSource,
  type ProxySettings,
  type ContractsClient,
  type InstalledPlugin,
  type InstallOutcome,
  type PluginConfigurationDetails,
  type PluginSettingValue,
  type Project,
  type RuntimeLogLevelStateResponse,
  type Session,
  type Skill,
  type Task,
  type Workflow,
  type WorkflowRun,
  type WorkflowSnapshot,
  type WorkflowSummary,
  type WorkflowVersion,
  type Workspace,
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
  workspaceId: string;
  createdAt: bigint;
  updatedAt: bigint;
}

/** Builds the public run payload for one mock workflow-run record. */
function mockWorkflowRun(record: MockWorkflowRunRecord): WorkflowRun {
  return {
    id: record.id,
    workspaceId: record.workspaceId,
    workflowId: record.workflowId,
    snapshotId: record.snapshotId,
    name: record.name,
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
  workspaces: Workspace[];
  tasks: Task[];
  sessions: Session[];
  agents: Agent[];
  skills: Skill[];
  installedPlugins: InstalledPlugin[];
  pluginConfigurations: Map<string, PluginConfigurationDetails>;
  /**
   * What the agent runtime reports reaching, which is what decides the agents the pickers offer.
   *
   * An agent missing from this list is one nothing supervises 鈥?an uninstalled plugin package.
   */
  agentRuntimeStatuses: AgentRuntimeStatus[];
  availablePlugins: AvailablePlugin[];
  /** README text served for one marketplace listing keyed by plugin id. */
  pluginReadmes: Map<string, string>;
  availablePluginsUpdatedAt: bigint;
  marketplaceSources: MarketplaceSource[];
  proxySettings: ProxySettings | null;
  /**
   * The package a local `.orax` import should materialize; `null` rejects that import.
   * Undefined means imports are not configured and always fail in tests. A concrete target is
   * committed as an installed package that is immediately available.
   */
  importTarget?: InstalledPlugin | null;
  /**
   * The typed install/import outcome returned by the mock plugin commands. Defaults to
   * `installed`; a conflict test supplies `installed_with_command_conflict`.
   */
  installOutcome?: InstallOutcome;
  developerMode: { enabled: boolean };
  runtimeLogLevel: RuntimeLogLevelStateResponse;
  workflows: MockWorkflowRecord[];
  workflowRuns: MockWorkflowRunRecord[];
  /** Warm sessions handed out but not yet attached, keyed by session id. */
  warmSessions: Map<string, { agentRef: string; workspaceId: string }>;
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
 * Every agent this mock installation offers, supplied by an installed package and detected.
 *
 * Agents exist only because a package supplies them, so a test needs both halves to see one in a
 * picker: the installed package that names it, and a runtime status that reaches it. A test that
 * needs one unreachable overrides `agentRuntimeStatuses`; one that needs it gone entirely
 * overrides `installedPlugins` as well.
 */
const AGENT_PACKAGES: { agentRef: string; displayName: string }[] = [
  { agentRef: "ora-space.opencode", displayName: "OpenCode" },
  { agentRef: "ora-space.nga", displayName: "NGA" },
  { agentRef: "ora-space.codeagentcli", displayName: "CodeAgentCLI" },
  { agentRef: "ora-space.claude", displayName: "Claude Code" },
  { agentRef: "ora-space.codex", displayName: "Codex" },
];

/** Builds the installed-package record one seeded agent is supplied by. */
function agentPackage(agentRef: string, displayName: string): InstalledPlugin {
  return {
    id: `official/${agentRef}`,
    namespace: "official",
    name: agentRef,
    displayName,
    version: "1.0.0",
    description: `${displayName} agent`,
    homepage: null,
    license: null,
    kind: "agent",
    agentDisplayName: displayName,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "running",
  };
}

/** Creates a fresh in-memory mock state with no records. */
export function createMockClientState(): MockClientState {
  return {
    projects: [],
    workspaces: [],
    tasks: [],
    sessions: [],
    agents: [],
    skills: [],
    installedPlugins: AGENT_PACKAGES.map((agent) =>
      agentPackage(agent.agentRef, agent.displayName),
    ),
    pluginConfigurations: new Map(),
    agentRuntimeStatuses: AGENT_PACKAGES.map((agent) => ({
      agentRef: agent.agentRef,
      status: "ready",
    })),
    availablePlugins: [],
    availablePluginsUpdatedAt: 0n,
    pluginReadmes: new Map(),
    marketplaceSources: [],
    proxySettings: null,
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

/** Materializes one installed plugin from a marketplace listing for mock install tests. */
function installedFromAvailable(available: AvailablePlugin): InstalledPlugin {
  const shared = {
    id: available.id,
    namespace: available.namespace,
    name: available.name,
    displayName: available.name,
    version: available.version,
    description: available.description,
    homepage: null,
    license: null,
    logo: available.logo,
    installationValidity: { validity: "valid" as const },
    configuration: { state: "not_declared" as const },
    runtime: "stopped" as const,
  };
  if (available.kind === "hook") {
    return {
      ...shared,
      kind: "hook",
      protocol: "rtk-rewrite-v1",
      command: "rtk",
      target: "x86_64-pc-windows-msvc",
      toolVersion: "0.45.0",
    };
  }
  return {
    ...shared,
    kind: "agent",
    agentDisplayName: available.name,
  };
}

/** Returns or creates the mock project's canonical Workspace projection. */
function mainWorkspaceFor(
  state: MockClientState,
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
function visibleWorkspaces(state: MockClientState): Workspace[] {
  for (const project of state.projects) {
    mainWorkspaceFor(state, project.id);
  }
  return [...state.workspaces];
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
        };
        state.projects.push(project);
        mainWorkspaceFor(state, project.id);
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
    workspace: {
      list: async () => ({ workspaces: visibleWorkspaces(state) }),
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
    task: {
      list: async () => ({ tasks: [...state.tasks] }),
      get: async (req) => ({
        task: state.tasks.find((t) => t.id === req.taskId)!,
      }),
      create: async (req) => {
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
      update: async (req) => {
        const idx = state.tasks.findIndex((t) => t.id === req.taskId);
        if (idx < 0) throw new Error(`task ${req.taskId} not found`);
        const updated: Task = {
          ...state.tasks[idx]!,
          title: req.title,
        };
        state.tasks[idx] = updated;
        return { task: updated };
      },
      delete: async (req) => {
        const idx = state.tasks.findIndex((t) => t.id === req.taskId);
        const workspaceId =
          idx >= 0 ? state.tasks[idx]!.workspaceId : `workspace-${req.taskId}`;
        if (idx >= 0) state.tasks.splice(idx, 1);
        return { taskId: req.taskId, workspaceId };
      },
      getWorkspace: async (req) => ({
        workspace: {
          rootPath: `/worktrees/${req.taskId}`,
          branchName: `task/${req.taskId}`,
        },
      }),
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
        const workspaceId = req.target.workspaceId;
        state.warmSessions.set(sessionId, {
          agentRef: req.agentRef,
          workspaceId,
        });
        const perCli = state.warmModelsByCli?.[req.agentRef];
        return {
          sessionId,
          workspaceId,
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
          workspaceId:
            state.warmSessions.get(req.sessionId)?.workspaceId ??
            req.workspaceId,
          agentRef:
            state.warmSessions.get(req.sessionId)?.agentRef ??
            "ora-space.opencode",
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
      getConfiguration: async (req) => {
        const configuration = state.pluginConfigurations.get(req.pluginId);
        if (configuration === undefined)
          throw new Error(`plugin configuration ${req.pluginId} not found`);
        return { configuration: structuredClone(configuration) };
      },
      saveConfiguration: async (req) => ({
        configuration: structuredClone(commitPluginConfiguration(state, req)),
      }),
      resetConfiguration: async (req) => ({
        configuration: structuredClone(
          commitPluginConfiguration(state, {
            pluginId: req.pluginId,
            expectedRevision:
              req.mode === "reset_all" ? req.expectedRevision : 0n,
            declarationFingerprint: req.declarationFingerprint,
            values: {},
          }),
        ),
      }),
      listAvailable: async () => ({
        updatedAt: state.availablePluginsUpdatedAt,
        plugins: [...state.availablePlugins],
      }),
      listSources: async () => ({ sources: [...state.marketplaceSources] }),
      addSource: async (req) => {
        if (state.marketplaceSources.some((source) => source.url === req.url))
          throw new Error(`marketplace source ${req.url} already exists`);
        const source = {
          url: req.url,
          branch: req.branch,
          useProxy: req.useProxy,
        };
        state.marketplaceSources.push(source);
        return { sources: [...state.marketplaceSources] };
      },
      deleteSource: async (req) => {
        const idx = state.marketplaceSources.findIndex(
          (source) => source.url === req.url,
        );
        if (idx < 0) throw new Error(`marketplace source ${req.url} not found`);
        state.marketplaceSources.splice(idx, 1);
        return { sources: [...state.marketplaceSources] };
      },
      updateSource: async (req) => {
        const source = state.marketplaceSources.find(
          (candidate) => candidate.url === req.url,
        );
        if (source === undefined)
          throw new Error(`marketplace source ${req.url} not found`);
        source.useProxy = req.useProxy;
        return { sources: [...state.marketplaceSources] };
      },
      syncAvailable: async () => ({
        updatedAt: state.availablePluginsUpdatedAt,
        plugins: [...state.availablePlugins],
      }),
      readReadme: async (req) => ({
        readme: state.pluginReadmes.get(req.pluginId) ?? null,
      }),
      scan: async () => ({ plugins: [...state.installedPlugins] }),
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
        if (req.dataDisposition === "delete")
          state.pluginConfigurations.delete(req.pluginId);
        return { pluginId: req.pluginId };
      },
      import: async (req) => {
        const target = state.importTarget;
        if (target === undefined)
          throw new Error(`import not configured for ${req.path}`);
        if (target === null) throw new Error(`import failed for ${req.path}`);
        const outcome = state.installOutcome ?? {
          state: "installed" as const,
        };
        state.installedPlugins.push({ ...target });
        return {
          pluginId: target.id,
          outcome,
        };
      },
      install: async (req) => {
        const available = state.availablePlugins.find(
          (p) => p.id === req.pluginId,
        );
        if (!available)
          throw new Error(`available plugin ${req.pluginId} not found`);
        const outcome = state.installOutcome ?? {
          state: "installed" as const,
        };
        state.installedPlugins.push(installedFromAvailable(available));
        return {
          pluginId: req.pluginId,
          outcome,
        };
      },
      update: async (req) => {
        const available = state.availablePlugins.find(
          (p) => p.id === req.pluginId,
        );
        if (!available)
          throw new Error(`available plugin ${req.pluginId} not found`);
        const installed = state.installedPlugins.find(
          (p) => p.id === req.pluginId,
        );
        if (!installed)
          throw new Error(`installed plugin ${req.pluginId} not found`);
        installed.version = available.version;
        installed.description = available.description;
        installed.logo = available.logo;
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
          source: { kind: "local" } as const,
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
          source: { kind: "local" } as const,
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
    proxy: {
      get: async () => ({ settings: state.proxySettings }),
      set: async (req) => {
        state.proxySettings = structuredClone(req.settings);
        return { settings: state.proxySettings };
      },
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
        state.workflows.unshift({ workflow, draft, published: [] });
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
          workspaceId: req.workspaceId,
          workflowId: req.workflowId,
          snapshotId: "snap-1",
          name: req.name ?? "",
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
          projectId:
            visibleWorkspaces(state).find(
              (workspace) => workspace.id === req.workspaceId,
            )?.projectId ?? "",
          workspaceId: req.workspaceId,
          workflowId: req.workflowId,
          snapshotId: run.snapshotId,
          name: req.name ?? "",
          status: "pending",
          createdAt: now,
          updatedAt: now,
        });
        return { run };
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
            workspaceId: record.workspaceId,
            workflowId: record.workflowId,
            snapshotId: record.snapshotId,
            name: record.name,
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
          workspaceId: record.workspaceId,
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
            workspaceId: record.workspaceId,
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
            workspaceId: record.workspaceId,
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
      rename: async (req) => {
        const record = state.workflowRuns.find(
          (candidate) => candidate.id === req.runId,
        );
        if (record === undefined)
          throw new Error(`workflow run ${req.runId} not found`);
        record.name = req.name;
        return { run: mockWorkflowRun(record) };
      },
    },
  };
}

/**
 * Commits one editor write into in-memory mock state.
 *
 * Save and reset share this so completeness and list-facing summaries cannot drift
 * between the two mock endpoints.
 */
function commitPluginConfiguration(
  state: MockClientState,
  req: {
    pluginId: string;
    expectedRevision: bigint;
    declarationFingerprint: string;
    values: { [key in string]: PluginSettingValue };
  },
): PluginConfigurationDetails {
  const current = state.pluginConfigurations.get(req.pluginId);
  if (current === undefined)
    throw new Error(`plugin configuration ${req.pluginId} not found`);
  if (req.declarationFingerprint !== current.declarationFingerprint)
    throw configurationWriteConflict(
      "plugin_configuration_declaration_changed",
    );
  if (req.expectedRevision !== current.revision)
    throw configurationWriteConflict("configuration_revision_conflict");
  const settings = current.settings.map((field) => {
    const storedValue = req.values[field.declaration.id];
    if (storedValue !== undefined)
      return {
        ...field,
        storedValue,
        effectiveValue: storedValue,
        source: "stored" as const,
        valueErrorCode: null,
      };
    return {
      ...field,
      storedValue: null,
      effectiveValue: field.declaration.default,
      source:
        field.declaration.default === null
          ? ("absent" as const)
          : ("default" as const),
      valueErrorCode: null,
    };
  });
  // Mock never injects valueErrorCode; incompleteness is only a missing required value.
  const incomplete = settings.some(
    (field) =>
      field.declaration.required &&
      (field.effectiveValue === null ||
        (typeof field.effectiveValue === "string" &&
          field.effectiveValue.trim() === "")),
  );
  const configuration: PluginConfigurationDetails = {
    ...current,
    revision: current.revision + 1n,
    settings,
    summary: {
      state: "available",
      completeness: incomplete ? "incomplete" : "complete",
    },
  };
  state.pluginConfigurations.set(req.pluginId, configuration);
  const plugin = state.installedPlugins.find(
    (candidate) => candidate.id === req.pluginId,
  );
  if (plugin !== undefined) {
    plugin.configuration = {
      state: "available",
      completeness: incomplete ? "incomplete" : "complete",
    };
  }
  return configuration;
}

/** Mirrors the two optimistic-concurrency failures returned by the desktop contract. */
function configurationWriteConflict(
  code:
    | "configuration_revision_conflict"
    | "plugin_configuration_declaration_changed",
): RemoteContractError {
  return new RemoteContractError(
    {
      code,
      params: {},
      requestId: "00000000-0000-4000-8000-000000000001",
    },
    null,
  );
}
