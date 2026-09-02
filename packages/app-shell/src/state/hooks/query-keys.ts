import type { WorkspaceDiffScope } from "@ora/contracts";

/**
 * Centralised react-query cache keys for the app shell.
 *
 * Keeping keys in one place lets mutations invalidate exactly the queries they
 * share data with, without scattering string literals across hook files.
 */
export const queryKeys = {
  projects: ["projects"] as const,
  workspaces: ["workspaces"] as const,
  projectBranches: (projectId: string) =>
    ["project-branches", projectId] as const,
  tasks: ["tasks"] as const,
  sessions: ["sessions"] as const,
  agents: ["agents"] as const,
  skills: ["skills"] as const,
  availablePlugins: ["available-plugins"] as const,
  pluginReadme: (pluginId: string) => ["plugin-readme", pluginId] as const,
  marketplaceSources: ["marketplace-sources"] as const,
  installedPlugins: ["installed-plugins"] as const,
  pluginConfiguration: (pluginId: string) =>
    ["plugin-configuration", pluginId] as const,
  developerMode: ["developer-mode"] as const,
  runtimeLogLevel: ["runtime-log-level"] as const,
  proxySettings: ["proxy-settings"] as const,
  gitIdentity: ["gitIdentity"] as const,
  /** Project 鈫?mounted graph workflow definitions (mock host). */
  workflowMounts: (projectId: string) => ["workflowMounts", projectId] as const,
  /** Definition 鈫?projects that already mount it. */
  workflowMountsByDefinition: (definitionId: string) =>
    ["workflowMountsByDefinition", definitionId] as const,
  /** Project 鈫?GraphWorkflowRun list (mock run repo). */
  workflowRuns: (projectId: string) => ["workflowRuns", projectId] as const,
  workflowRun: (runId: string) => ["workflowRun", runId] as const,
  /** Artifacts produced by one graph workflow run. */
  workflowArtifacts: (runId: string) => ["workflowArtifacts", runId] as const,
  agentRuntimeStatus: ["agentRuntimeStatus"] as const,
  taskWorkspace: (taskId: string) => ["task-workspace", taskId] as const,
  workspaceCwd: (workspaceId: string) =>
    ["workspace-cwd", workspaceId] as const,
  workspaceDiffs: (workspaceId: string) =>
    ["workspace-diff", workspaceId] as const,
  workspaceDiff: (workspaceId: string, scope: WorkspaceDiffScope) =>
    ["workspace-diff", workspaceId, scope] as const,
  workspaceFiles: (taskId: string) => ["workspace-files", taskId] as const,
  workspaceDirectory: (taskId: string, path: string) =>
    ["workspace-files", taskId, "directory", path] as const,
  workspaceFile: (taskId: string, path: string) =>
    ["workspace-files", taskId, "file", path] as const,
  workspaceSearch: (taskId: string, kind: string, query: string) =>
    ["workspace-files", taskId, "search", kind, query] as const,
  projectFiles: (projectId: string) => ["project-files", projectId] as const,
  projectDirectory: (projectId: string, path: string) =>
    ["project-files", projectId, "directory", path] as const,
  projectFile: (projectId: string, path: string) =>
    ["project-files", projectId, "file", path] as const,
  projectSearch: (projectId: string, kind: string, query: string) =>
    ["project-files", projectId, "search", kind, query] as const,
  agentModels: (agentRef: string | null, workspaceId: string | null) =>
    ["agentModels", agentRef ?? "none", workspaceId ?? "none"] as const,
  /** Every client-side discovery query backed by one agent plugin. */
  agentModelsForAgent: (agentRef: string) => ["agentModels", agentRef] as const,
};

export type WorkspaceQueryKey =
  | readonly ["projects"]
  | readonly ["workspaces"]
  | readonly ["tasks"]
  | readonly ["sessions"];
