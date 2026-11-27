import {
  type ProxyMode,
  type ProxySettings,
  type RuntimeLogLevelStateResponse,
} from "@ora/contracts";
import type { TestHandlers } from "../contracts-transport";

/** Mutable records owned by the settings test adapter. */
export interface SettingsMemoryState {
  proxySettings: ProxySettings | null;
  proxyMode?: ProxyMode;
  proxyCheckUrl: string;
  developerMode: { enabled: boolean };
  runtimeLogLevel: RuntimeLogLevelStateResponse;
}

/** Creates an independent settings memory fixture. */
export function createSettingsMemory(): SettingsMemoryState {
  return {
    proxySettings: null,
    proxyCheckUrl: "https://google.com",
    developerMode: { enabled: false },
    runtimeLogLevel: {
      configuredLevel: "info",
      effectiveLevel: "info",
    },
  };
}

/** Registers only the settings operations explicitly requested by a fixture. */
export function settingsHandlers(state: SettingsMemoryState) {
  return {
    getProxyCheckUrl: async () => ({ url: state.proxyCheckUrl }),
    setProxyCheckUrl: async ({ url }) => {
      const parsed = new URL(url);
      if (!["http:", "https:"].includes(parsed.protocol)) {
        throw new Error("Invalid probe URL");
      }
      state.proxyCheckUrl = url.trim();
      return { url: state.proxyCheckUrl };
    },
    getProxySettings: async ({ loadManual }) => ({
      settings:
        loadManual || !state.proxyMode || state.proxyMode === "manual"
          ? state.proxySettings
          : null,
      ...(state.proxyMode ? { mode: state.proxyMode } : {}),
    }),
    setProxySettings: async (req) => {
      state.proxyMode = req.mode ?? "manual";
      if (state.proxyMode === "manual") {
        state.proxySettings = structuredClone(req.settings);
      }
      return {
        settings: state.proxyMode === "manual" ? state.proxySettings : null,
        mode: state.proxyMode,
      };
    },
    clearProxySettings: async () => {
      state.proxySettings = null;
      state.proxyMode = "system";
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
      };
      return { ...state.runtimeLogLevel };
    },
  } satisfies TestHandlers;
}
