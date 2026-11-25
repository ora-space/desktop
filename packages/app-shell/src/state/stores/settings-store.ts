import { create } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";
export type ThemeMode = "system" | "light" | "dark";
export type ApprovalPolicy = "always" | "risky" | "trusted";
export type HistoryRetention = "30-days" | "90-days" | "forever";

export interface SettingsPreferences {
  theme: ThemeMode;
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
      // Preserve old defaults because they may be explicit user selections.
      migrate: (persisted) => persisted as SettingsState,
      // Tolerate partial/corrupt persisted state by merging over defaults. A stored agent
      // identity is carried forward unexamined: agents arrive with installed plugins, so this
      // build cannot know which identities are real, and the pickers already resolve a stored one
      // against the live runtime before offering it or discovering its models.
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

/** The media query that decides what "system" resolves to. */
export const DARK_THEME_QUERY = "(prefers-color-scheme: dark)";

/**
 * Collapses the three-state preference into the two states anything actually renders in.
 *
 * Exported because more than the DOM applier needs the answer: assets that ship a light and a
 * dark file have to pick one, and they must pick it the same way the stylesheet does or the icon
 * and its surroundings disagree.
 */
export function isDarkTheme(theme: ThemeMode): boolean {
  return (
    theme === "dark" ||
    (theme === "system" && window.matchMedia(DARK_THEME_QUERY).matches)
  );
}

/** Applies the active theme to <html> so Tailwind variant classes resolve correctly. */
export type ThemeApplier = (settings: SettingsPreferences) => void;

let themeApplier: ThemeApplier = defaultThemeApplier;

function defaultThemeApplier(settings: SettingsPreferences): void {
  document.documentElement.classList.toggle(
    "dark",
    isDarkTheme(settings.theme),
  );
  document.documentElement.dataset.theme = settings.theme;
}

let themeSubscriptionCleanup: (() => void) | null = null;

/**
 * Starts a module-level subscription that mirrors settings.theme onto the document.
 * Returns a cleanup function that tears down both the store listener and the matchMedia listener.
 */
export function startThemeSubscription(): () => void {
  if (themeSubscriptionCleanup) return themeSubscriptionCleanup;

  const apply = () => themeApplier(useSettingsStore.getState().settings);
  apply();

  const unsubscribeStore = useSettingsStore.subscribe((state) =>
    themeApplier(state.settings),
  );
  const media = window.matchMedia(DARK_THEME_QUERY);
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
