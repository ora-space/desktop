import { create } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";

export type ThemeMode = "system" | "light" | "dark";
export type InterfaceDensity = "comfortable" | "compact";
export type ModelProvider = "openai" | "anthropic" | "local";
export type ApprovalPolicy = "always" | "risky" | "trusted";
export type HistoryRetention = "30-days" | "90-days" | "forever";

export interface SettingsPreferences {
  theme: ThemeMode;
  density: InterfaceDensity;
  provider: ModelProvider;
  model: string;
  approvalPolicy: ApprovalPolicy;
  terminalAccess: boolean;
  fileWriteAccess: boolean;
  networkAccess: boolean;
  commandTimeout: string;
  historyRetention: HistoryRetention;
  diagnostics: boolean;
}

const SETTINGS_STORAGE_KEY = "ora.settings.v1";

export const DEFAULT_SETTINGS: SettingsPreferences = {
  theme: "system",
  density: "comfortable",
  provider: "openai",
  model: "gpt-5.1-codex",
  approvalPolicy: "trusted",
  terminalAccess: true,
  fileWriteAccess: true,
  networkAccess: false,
  commandTimeout: "120",
  historyRetention: "30-days",
  diagnostics: false,
};

interface SettingsState {
  settings: SettingsPreferences;
  updateSettings: (patch: Partial<SettingsPreferences>) => void;
  resetSettings: () => void;
}

/** Persisted prototype preferences, mirrored to localStorage via zustand persist. */
export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      settings: DEFAULT_SETTINGS,
      updateSettings: (patch) =>
        set((state) => ({ settings: { ...state.settings, ...patch } })),
      resetSettings: () => set({ settings: DEFAULT_SETTINGS }),
    }),
    {
      name: SETTINGS_STORAGE_KEY,
      storage: createJSONStorage(() => window.localStorage),
      // Tolerate partial/corrupt persisted state by merging over defaults.
      merge: (persisted, current) => {
        const persistedSettings = (persisted as Partial<SettingsState> | undefined)?.settings;
        return {
          ...current,
          settings: { ...DEFAULT_SETTINGS, ...(persistedSettings ?? {}) },
        };
      },
    },
  ),
);

/** Applies the active theme/density to <html> so Tailwind variant classes resolve correctly. */
export type ThemeApplier = (settings: SettingsPreferences) => void;

let themeApplier: ThemeApplier = defaultThemeApplier;

function defaultThemeApplier(settings: SettingsPreferences): void {
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  const dark = settings.theme === "dark" || (settings.theme === "system" && media.matches);
  document.documentElement.classList.toggle("dark", dark);
  document.documentElement.dataset.theme = settings.theme;
  document.documentElement.dataset.density = settings.density;
}

let themeSubscriptionCleanup: (() => void) | null = null;

/**
 * Starts a module-level subscription that mirrors settings.theme/density onto the document.
 * Returns a cleanup function that tears down both the store listener and the matchMedia listener.
 */
export function startThemeSubscription(): () => void {
  if (themeSubscriptionCleanup) return themeSubscriptionCleanup;

  const apply = () => themeApplier(useSettingsStore.getState().settings);
  apply();

  const unsubscribeStore = useSettingsStore.subscribe((state) => themeApplier(state.settings));
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  media.addEventListener("change", apply);

  themeSubscriptionCleanup = () => {
    unsubscribeStore();
    media.removeEventListener("change", apply);
    themeSubscriptionCleanup = null;
  };
  return themeSubscriptionCleanup;
}

/** Test-only: replaces the DOM side-effect applier so unit tests can assert what would be written. */
export function setThemeApplier(applier: ThemeApplier): void {
  themeApplier = applier;
}
