import {
  RemoteContractError,
  type AvailablePlugin,
  type MarketplaceSource,
  type InstalledPlugin,
  type InstallOutcome,
  type PluginConfigurationDetails,
  type PluginSettingValue,
} from "@ora/contracts";
import type { TestHandlers } from "../contracts-transport";
import { seededAgentPackages } from "./agent-packages";

/** Mutable records owned by the plugins test adapter. */
export interface PluginMemoryState {
  installedPlugins: InstalledPlugin[];
  pluginConfigurations: Map<string, PluginConfigurationDetails>;
  availablePlugins: AvailablePlugin[];
  /** README text served for one marketplace listing keyed by plugin id. */
  pluginReadmes: Map<string, string>;
  availablePluginsUpdatedAt: bigint;
  marketplaceSources: MarketplaceSource[];
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
}

/** Creates an independent plugins memory fixture. */
export function createPluginMemory(): PluginMemoryState {
  const installedPlugins = seededAgentPackages();
  return {
    installedPlugins,
    pluginConfigurations: new Map(),
    availablePlugins: [],
    pluginReadmes: new Map(),
    availablePluginsUpdatedAt: 0n,
    marketplaceSources: [],
  };
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

/**
 * Commits one editor write into in-memory mock state.
 *
 * Save and reset share this so completeness and list-facing summaries cannot drift
 * between the two mock endpoints.
 */
function commitPluginConfiguration(
  state: PluginMemoryState,
  req: {
    pluginId: string;
    expectedRevision: bigint;
    declarationFingerprint: string;
    values: { [key in string]: PluginSettingValue };
    preserveSettingIds: string[];
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
    if (req.preserveSettingIds.includes(field.declaration.id)) return field;
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

/** Registers only the plugins operations explicitly requested by a fixture. */
export function pluginHandlers(state: PluginMemoryState) {
  return {
    listInstalledPlugins: async () => ({
      plugins: [...state.installedPlugins],
    }),
    getPluginConfiguration: async (req) => {
      const configuration = state.pluginConfigurations.get(req.pluginId);
      if (configuration === undefined)
        throw new Error(`plugin configuration ${req.pluginId} not found`);
      return { configuration: structuredClone(configuration) };
    },
    savePluginConfiguration: async (req) => ({
      configuration: structuredClone(commitPluginConfiguration(state, req)),
    }),
    resetPluginConfiguration: async (req) => ({
      configuration: structuredClone(
        commitPluginConfiguration(state, {
          pluginId: req.pluginId,
          expectedRevision:
            req.mode === "reset_all" ? req.expectedRevision : 0n,
          declarationFingerprint: req.declarationFingerprint,
          values: {},
          preserveSettingIds: [],
        }),
      ),
    }),
    listAvailablePlugins: async () => ({
      updatedAt: state.availablePluginsUpdatedAt,
      plugins: [...state.availablePlugins],
    }),
    listMarketplaceSources: async () => ({
      sources: [...state.marketplaceSources],
    }),
    addMarketplaceSource: async (req) => {
      if (state.marketplaceSources.some((source) => source.url === req.url))
        throw new Error(`marketplace source ${req.url} already exists`);
      const source = {
        url: req.url,
        branch: req.branch,
        useProxy: req.useProxy,
        enabled: true,
        artifactRetrieval: { type: "direct_https" } as const,
      };
      state.marketplaceSources.push(source);
      return { sources: [...state.marketplaceSources] };
    },
    deleteMarketplaceSource: async (req) => {
      const idx = state.marketplaceSources.findIndex(
        (source) => source.url === req.url,
      );
      if (idx < 0) throw new Error(`marketplace source ${req.url} not found`);
      state.marketplaceSources.splice(idx, 1);
      return { sources: [...state.marketplaceSources] };
    },
    updateMarketplaceSource: async (req) => {
      const idx = state.marketplaceSources.findIndex(
        (candidate) => candidate.url === req.url,
      );
      if (idx < 0) throw new Error(`marketplace source ${req.url} not found`);
      if (
        req.newUrl !== req.url &&
        state.marketplaceSources.some((source) => source.url === req.newUrl)
      ) {
        throw new Error(`marketplace source ${req.newUrl} already exists`);
      }
      state.marketplaceSources[idx] = {
        url: req.newUrl,
        branch: req.branch,
        useProxy: req.useProxy,
        enabled: req.enabled,
        artifactRetrieval:
          req.artifactRetrieval.type === "direct_https"
            ? req.artifactRetrieval
            : {
                type: "s3_sigv4" as const,
                endpoint: req.artifactRetrieval.endpoint,
                bucket: req.artifactRetrieval.bucket,
                region: req.artifactRetrieval.region,
              },
      };
      return { sources: [...state.marketplaceSources] };
    },
    syncAvailablePlugins: async () => ({
      updatedAt: state.availablePluginsUpdatedAt,
      plugins: [...state.availablePlugins],
    }),
    readPluginReadme: async (req) => ({
      readme: state.pluginReadmes.get(req.pluginId) ?? null,
    }),
    scanPlugins: async () => ({ plugins: [...state.installedPlugins] }),
    activatePlugin: async (req) => {
      const plugin = state.installedPlugins.find((p) => p.id === req.pluginId);
      if (!plugin)
        throw new Error(`installed plugin ${req.pluginId} not found`);
      plugin.runtime = "running";
      return { plugin };
    },
    stopPlugin: async (req) => {
      const plugin = state.installedPlugins.find((p) => p.id === req.pluginId);
      if (!plugin)
        throw new Error(`installed plugin ${req.pluginId} not found`);
      plugin.runtime = "stopped";
      return { plugin };
    },
    uninstallPlugin: async (req) => {
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
    importPlugin: async (req) => {
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
    installPlugin: async (req) => {
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
    updatePlugin: async (req) => {
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
  } satisfies TestHandlers;
}
