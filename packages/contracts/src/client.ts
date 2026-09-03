import {
  endpoints,
  type EndpointOperation,
  type RequestByOperation,
  type ResponseByOperation,
} from "./endpoints.js";
import type {
  ContractCallOptions,
  ContractTransport,
  ContractTransportRequest,
} from "./transport.js";

type ClientOperation<Operation extends EndpointOperation> = (
  request: RequestByOperation[Operation],
  options?: ContractCallOptions,
) => (typeof endpoints)[Operation]["responseMode"] extends "stream"
  ? AsyncIterable<ResponseByOperation[Operation]>
  : Promise<ResponseByOperation[Operation]>;

/**
 * Namespaces declared by the endpoint manifest. Sourced from `endpoints` so
 * adding a route in Rust without re-exporting contracts cannot leave the
 * hand-written client silently out of sync.
 */
type EndpointNamespace = (typeof endpoints)[EndpointOperation]["namespace"];

/**
 * Typed shape of the contracts client, derived from the generated `endpoints`
 * manifest. Each endpoint in `ora-contracts` declares a `namespace` and
 * `memberName`; this type re-groups the flat `EndpointOperation` union into
 * the nested shape used at the call site (`client.project.create`).
 *
 * Because the shape is derived from the manifest and `createContractsClient`
 * returns an object literal checked against this type, adding a route in Rust
 * without updating `client.ts` fails `tsc` with a missing-property error,
 * keeping the hand-written client in compile-time lockstep with the routes.
 */
export type ContractsClient = {
  [Namespace in EndpointNamespace]: {
    [
      Operation in EndpointOperation as (typeof endpoints)[Operation]["namespace"] extends Namespace
        ? (typeof endpoints)[Operation]["memberName"]
        : never
    ]: ClientOperation<Operation>;
  };
};

export function createContractsClient(
  transport: ContractTransport,
): ContractsClient {
  return {
    project: {
      create: (request, options) =>
        executeOperation("createProject", request, transport, options),
      get: (request, options) =>
        executeOperation("getProject", request, transport, options),
      list: (request, options) =>
        executeOperation("listProjects", request, transport, options),
      listBranches: (request, options) =>
        executeOperation("listProjectBranches", request, transport, options),
      update: (request, options) =>
        executeOperation("updateProject", request, transport, options),
      delete: (request, options) =>
        executeOperation("deleteProject", request, transport, options),
    },
    workspace: {
      list: (request, options) =>
        executeOperation("listWorkspaces", request, transport, options),
      getDiff: (request, options) =>
        executeOperation("getWorkspaceDiff", request, transport, options),
      commitChanges: (request, options) =>
        executeOperation("commitWorkspaceChanges", request, transport, options),
      pushBranch: (request, options) =>
        executeOperation("pushWorkspaceBranch", request, transport, options),
    },
    task: {
      create: (request, options) =>
        executeOperation("createTask", request, transport, options),
      get: (request, options) =>
        executeOperation("getTask", request, transport, options),
      getWorkspace: (request, options) =>
        executeOperation("getTaskWorkspace", request, transport, options),
      list: (request, options) =>
        executeOperation("listTasks", request, transport, options),
      update: (request, options) =>
        executeOperation("updateTask", request, transport, options),
      delete: (request, options) =>
        executeOperation("deleteTask", request, transport, options),
    },
    session: {
      start: (request, options) =>
        executeOperation("startSession", request, transport, options),
      setConfig: (request, options) =>
        executeOperation("setSessionConfig", request, transport, options),
      get: (request, options) =>
        executeOperation("getSession", request, transport, options),
      list: (request, options) =>
        executeOperation("listSessions", request, transport, options),
      load: (request, options) =>
        executeStreamOperation("loadSession", request, transport, options),
      prompt: (request, options) =>
        executeStreamOperation("promptSession", request, transport, options),
      respondToPermission: (request, options) =>
        executeOperation(
          "respondToSessionPermission",
          request,
          transport,
          options,
        ),
      cancelPrompt: (request, options) =>
        executeOperation("cancelSessionPrompt", request, transport, options),
      stop: (request, options) =>
        executeOperation("stopSession", request, transport, options),
      switchAgent: (request, options) =>
        executeOperation("switchSessionAgent", request, transport, options),
      resumeHistory: (request, options) =>
        executeOperation("resumeSessionHistory", request, transport, options),
      delete: (request, options) =>
        executeOperation("deleteSession", request, transport, options),
      rename: (request, options) =>
        executeOperation("renameSession", request, transport, options),
    },
    appEvents: {
      watch: (request, options) =>
        executeStreamOperation("watchAppEvents", request, transport, options),
    },
    agentRuntime: {
      getStatus: (request, options) =>
        executeOperation("getAgentRuntimeStatus", request, transport, options),
      listModels: (request, options) =>
        executeOperation("listAgentModels", request, transport, options),
    },
    effect: {
      getTargetStatus: (request, options) =>
        executeOperation("getEffectTargetStatus", request, transport, options),
    },
    skill: {
      create: (request, options) =>
        executeOperation("createSkill", request, transport, options),
      get: (request, options) =>
        executeOperation("getSkill", request, transport, options),
      list: (request, options) =>
        executeOperation("listSkills", request, transport, options),
      update: (request, options) =>
        executeOperation("updateSkill", request, transport, options),
      delete: (request, options) =>
        executeOperation("deleteSkill", request, transport, options),
    },
    skillImport: {
      prepare: (request, options) =>
        executeOperation("prepareSkillImport", request, transport, options),
      get: (request, options) =>
        executeOperation("getSkillImport", request, transport, options),
      commit: (request, options) =>
        executeOperation("commitSkillImport", request, transport, options),
      cancel: (request, options) =>
        executeOperation("cancelSkillImport", request, transport, options),
    },
    agent: {
      create: (request, options) =>
        executeOperation("createAgent", request, transport, options),
      get: (request, options) =>
        executeOperation("getAgent", request, transport, options),
      list: (request, options) =>
        executeOperation("listAgents", request, transport, options),
      update: (request, options) =>
        executeOperation("updateAgent", request, transport, options),
      delete: (request, options) =>
        executeOperation("deleteAgent", request, transport, options),
    },
    agentImport: {
      prepare: (request, options) =>
        executeOperation("prepareAgentImport", request, transport, options),
      commit: (request, options) =>
        executeOperation("commitAgentImport", request, transport, options),
    },
    plugin: {
      listAvailable: (request, options) =>
        executeOperation("listAvailablePlugins", request, transport, options),
      syncAvailable: (request, options) =>
        executeOperation("syncAvailablePlugins", request, transport, options),
      readReadme: (request, options) =>
        executeOperation("readPluginReadme", request, transport, options),
      listSources: (request, options) =>
        executeOperation("listMarketplaceSources", request, transport, options),
      addSource: (request, options) =>
        executeOperation("addMarketplaceSource", request, transport, options),
      deleteSource: (request, options) =>
        executeOperation(
          "deleteMarketplaceSource",
          request,
          transport,
          options,
        ),
      updateSource: (request, options) =>
        executeOperation("updateMarketplaceSource", request, transport, options),

      listInstalled: (request, options) =>
        executeOperation("listInstalledPlugins", request, transport, options),
      getConfiguration: (request, options) =>
        executeOperation("getPluginConfiguration", request, transport, options),
      saveConfiguration: (request, options) =>
        executeOperation(
          "savePluginConfiguration",
          request,
          transport,
          options,
        ),
      resetConfiguration: (request, options) =>
        executeOperation(
          "resetPluginConfiguration",
          request,
          transport,
          options,
        ),
      scan: (request, options) =>
        executeOperation("scanPlugins", request, transport, options),
      activate: (request, options) =>
        executeOperation("activatePlugin", request, transport, options),
      stop: (request, options) =>
        executeOperation("stopPlugin", request, transport, options),
      uninstall: (request, options) =>
        executeOperation("uninstallPlugin", request, transport, options),
      install: (request, options) =>
        executeOperation("installPlugin", request, transport, options),
      update: (request, options) =>
        executeOperation("updatePlugin", request, transport, options),
      import: (request, options) =>
        executeOperation("importPlugin", request, transport, options),
    },
    proxy: {
      get: (request, options) =>
        executeOperation("getProxySettings", request, transport, options),
      set: (request, options) =>
        executeOperation("setProxySettings", request, transport, options),
      clear: (request, options) =>
        executeOperation("clearProxySettings", request, transport, options),
      check: (request, options) =>
        executeOperation("checkProxySettings", request, transport, options),
    },
    fileSystem: {
      listWorkspaceDirectory: (request, options) =>
        executeOperation("listWorkspaceDirectory", request, transport, options),
      readWorkspaceFile: (request, options) =>
        executeOperation("readWorkspaceFile", request, transport, options),
      searchWorkspace: (request, options) =>
        executeOperation("searchWorkspace", request, transport, options),
      watchWorkspace: (request, options) =>
        executeStreamOperation("watchWorkspace", request, transport, options),
      listProjectDirectory: (request, options) =>
        executeOperation("listProjectDirectory", request, transport, options),
      readProjectFile: (request, options) =>
        executeOperation("readProjectFile", request, transport, options),
      searchProject: (request, options) =>
        executeOperation("searchProject", request, transport, options),
      watchProject: (request, options) =>
        executeStreamOperation("watchProject", request, transport, options),
    },
    gitIdentity: {
      get: (request, options) =>
        executeOperation("getGitIdentity", request, transport, options),
    },
    developerMode: {
      get: (request, options) =>
        executeOperation("getDeveloperMode", request, transport, options),
      set: (request, options) =>
        executeOperation("setDeveloperMode", request, transport, options),
    },
    runtimeLogLevel: {
      get: (request, options) =>
        executeOperation("getRuntimeLogLevel", request, transport, options),
      set: (request, options) =>
        executeOperation("setRuntimeLogLevel", request, transport, options),
    },
    workflow: {
      create: (request, options) =>
        executeOperation("createWorkflow", request, transport, options),
      get: (request, options) =>
        executeOperation("getWorkflow", request, transport, options),
      list: (request, options) =>
        executeOperation("listWorkflows", request, transport, options),
      update: (request, options) =>
        executeOperation("updateWorkflow", request, transport, options),
      delete: (request, options) =>
        executeOperation("deleteWorkflow", request, transport, options),
      getDraft: (request, options) =>
        executeOperation("getDraft", request, transport, options),
      updateDraft: (request, options) =>
        executeOperation("updateDraft", request, transport, options),
      publish: (request, options) =>
        executeOperation("publishWorkflow", request, transport, options),
      rollback: (request, options) =>
        executeOperation("rollbackWorkflow", request, transport, options),
      activate: (request, options) =>
        executeOperation("activateWorkflow", request, transport, options),
      listVersions: (request, options) =>
        executeOperation("listVersions", request, transport, options),
      getVersion: (request, options) =>
        executeOperation("getVersion", request, transport, options),
      deleteSnapshot: (request, options) =>
        executeOperation("deleteSnapshot", request, transport, options),
      getSnapshot: (request, options) =>
        executeOperation("getWorkflowSnapshot", request, transport, options),
    },
    workflowRun: {
      create: (request, options) =>
        executeOperation("createWorkflowRun", request, transport, options),
      get: (request, options) =>
        executeOperation("getWorkflowRun", request, transport, options),
      list: (request, options) =>
        executeOperation("listWorkflowRuns", request, transport, options),
      listByWorkflow: (request, options) =>
        executeOperation(
          "listWorkflowRunsByWorkflow",
          request,
          transport,
          options,
        ),
      listNodeRuns: (request, options) =>
        executeOperation("listWorkflowNodeRuns", request, transport, options),
      delete: (request, options) =>
        executeOperation("deleteWorkflowRun", request, transport, options),
      rename: (request, options) =>
        executeOperation("renameWorkflowRun", request, transport, options),
      start: (request, options) =>
        executeOperation("startWorkflowRun", request, transport, options),
      cancel: (request, options) =>
        executeOperation("cancelWorkflowRun", request, transport, options),
      restart: (request, options) =>
        executeOperation("restartWorkflowRun", request, transport, options),
      updateInput: (request, options) =>
        executeOperation("updateWorkflowRunInput", request, transport, options),
      completeNode: (request, options) =>
        executeOperation("completeWorkflowNode", request, transport, options),
    },
  };
}

async function executeOperation<Operation extends EndpointOperation>(
  operation: Operation,
  request: RequestByOperation[Operation],
  transport: ContractTransport,
  options?: ContractCallOptions,
): Promise<ResponseByOperation[Operation]> {
  const endpoint = endpoints[operation];
  const transportRequest: ContractTransportRequest = {
    operationName: endpoint.operationName,
    request,
  };

  return transport.send<ResponseByOperation[Operation]>(
    transportRequest,
    options,
  );
}

/** Builds one typed request and delegates stream lifecycle to the selected transport. */
function executeStreamOperation<Operation extends EndpointOperation>(
  operation: Operation,
  request: RequestByOperation[Operation],
  transport: ContractTransport,
  options?: ContractCallOptions,
): AsyncIterable<ResponseByOperation[Operation]> {
  const endpoint = endpoints[operation];
  return transport.stream<ResponseByOperation[Operation]>(
    {
      operationName: endpoint.operationName,
      request,
    },
    options,
  );
}
