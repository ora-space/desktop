import {
  type ProxySettings,
  type RuntimeLogLevelStateResponse,
} from "@ora/contracts";
import type { TestHandlers } from "../contracts-transport";

/** Mutable records owned by the settings test adapter. */
export interface SettingsMemoryState {
  proxySettings: ProxySettings | null;
  developerMode: { enabled: boolean };
  runtimeLogLevel: RuntimeLogLevelStateResponse;
}

/** Creates an independent settings memory fixture. */
export function createSettingsMemory(): SettingsMemoryState {
  return {
    proxySettings: null,
    developerMode: { enabled: false },
    runtimeLogLevel: {
      configuredLevel: "info",
      effectiveLevel: "info",
      startupOverride: null,
    },
  };
}

/** Registers only the settings operations explicitly requested by a fixture. */
export function settingsHandlers(state: SettingsMemoryState) {
  return {
    getProxySettings: async () => ({ settings: state.proxySettings }),
    setProxySettings: async (req) => {
      state.proxySettings = structuredClone(req.settings);
      return { settings: state.proxySettings };
    },
    clearProxySettings: async () => {
      state.proxySettings = null;
      return { settings: null };
    },
    checkProxySettings: async () => ({
      outcome: "reachable" as const,
      status: 200,
    }),
    getDeveloperMode: async () => ({ ...state.developerMode }),
    setDeveloperMode: async (request) => {
      state.developerMode = { enabled: request.enabled };
      return { ...state.developerMode };
    },
    getRuntimeLogLevel: async () => ({ ...state.runtimeLogLevel }),
    setRuntimeLogLevel: async (request) => {
      state.runtimeLogLevel = {
        configuredLevel: request.level,
        effectiveLevel: request.level,
        startupOverride: state.runtimeLogLevel.startupOverride,
      };
      return { ...state.runtimeLogLevel };
    },
  } satisfies TestHandlers;
}
