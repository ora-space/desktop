import { create } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";
export type ThemeMode = "system" | "light" | "dark";
export type InterfaceDensity = "comfortable" | "compact";
export type ApprovalPolicy = "always" | "risky" | "trusted";
export type HistoryRetention = "30-days" | "90-days" | "forever";

export interface SettingsPreferences {
  theme: ThemeMode;
  density: InterfaceDensity;
  /**
   * Persisted namespaced identity of the agent the next untouched chat surface opens on.
   *
   * Deliberately an open string: which agents exist depends on installed plugins, so a stored
   * identity naming one that is no longer installed is an ordinary state. The pickers resolve it
   * against the live runtime rather than validating it here.
   */
  agentCli: string | null;
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
  agentCli: null,
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
      version: 2,
      storage: createJSONStorage(() => window.localStorage),
      // Version 1 persisted the implicit OpenCode default together with unrelated
      // preference changes, so that value cannot prove the user selected it. A
      // different CLI could only have been written by an explicit picker action
      // and is retained; the former default becomes the new unselected state.
      migrate: (persisted, version) => {
        if (version >= 2) return persisted as SettingsState;
        const previous = persisted as Partial<SettingsState> | undefined;
        const previousSettings = previous?.settings;
        if (previousSettings?.agentCli !== "ora-space.opencode")
          return persisted as SettingsState;
        return {
          ...previous,
          settings: { ...previousSettings, agentCli: null },
        } as SettingsState;
      },
      // Tolerate partial/corrupt persisted state by merging over defaults. A stored agent
      // identity is carried forward unexamined: agents arrive with installed plugins, so this
      // build cannot know which identities are real, and the pickers already resolve a stored one
      // against the live runtime before offering or warming it.
      merge: (persisted, current) => {
        const persistedSettings = (
          persisted as Partial<SettingsState> | undefined
        )?.settings;
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
  const dark =
    settings.theme === "dark" || (settings.theme === "system" && media.matches);
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

  const unsubscribeStore = useSettingsStore.subscribe((state) =>
    themeApplier(state.settings),
  );
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
