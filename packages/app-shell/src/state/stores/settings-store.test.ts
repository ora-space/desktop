import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  useSettingsStore,
  DEFAULT_SETTINGS,
  startThemeSubscription,
  setThemeApplier,
  type SettingsPreferences,
  type ThemeApplier,
} from "./settings-store";

const STORAGE_KEY = "ora.settings.v1";

beforeEach(() => {
  window.localStorage.clear();
  useSettingsStore.setState({ settings: { ...DEFAULT_SETTINGS } });
});

afterEach(() => {
  useSettingsStore.setState({ settings: { ...DEFAULT_SETTINGS } });
});

describe("useSettingsStore", () => {
  it("starts with default settings", () => {
    expect(useSettingsStore.getState().settings).toEqual(DEFAULT_SETTINGS);
  });

  it("merges a partial patch into settings", () => {
    useSettingsStore.getState().updateSettings({
      theme: "dark",
      model: "claude-sonnet-4.5",
      historyRetention: "30-days",
    });
    expect(useSettingsStore.getState().settings).toEqual({
      ...DEFAULT_SETTINGS,
      theme: "dark",
      model: "claude-sonnet-4.5",
      historyRetention: "30-days",
    });
  });

  it("resets settings back to defaults", () => {
    useSettingsStore.getState().updateSettings({ theme: "dark", diagnostics: true });
    useSettingsStore.getState().resetSettings();
    expect(useSettingsStore.getState().settings).toEqual(DEFAULT_SETTINGS);
  });

  it("persists settings to localStorage under the v1 key", () => {
    useSettingsStore.getState().updateSettings({ provider: "anthropic" });
    const raw = window.localStorage.getItem(STORAGE_KEY);
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!) as { state: { settings: SettingsPreferences } };
    expect(parsed.state.settings.provider).toBe("anthropic");
  });

  it("merges persisted partial settings over defaults via the merge strategy", () => {
    // Simulate a legacy/partial payload that only carries one field.
    window.localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ state: { settings: { theme: "light" } } }),
    );
    // Force rehydrate by reloading the persisted slice through the store's persist API.
    useSettingsStore.persist.rehydrate();
    expect(useSettingsStore.getState().settings).toEqual({ ...DEFAULT_SETTINGS, theme: "light" });
  });

  it("falls back to defaults when persisted JSON is corrupt", () => {
    window.localStorage.setItem(STORAGE_KEY, "{not json");
    useSettingsStore.persist.rehydrate();
    expect(useSettingsStore.getState().settings).toEqual(DEFAULT_SETTINGS);
  });
});

describe("startThemeSubscription", () => {
  let cleanup: (() => void) | null = null;
  let applied: SettingsPreferences[] = [];

  const recordingApplier: ThemeApplier = (settings) => {
    applied.push(settings);
  };

  beforeEach(() => {
    applied = [];
    setThemeApplier(recordingApplier);
  });

  afterEach(() => {
    cleanup?.();
    cleanup = null;
    setThemeApplier((settings) => {
      const media = window.matchMedia("(prefers-color-scheme: dark)");
      const dark = settings.theme === "dark" || (settings.theme === "system" && media.matches);
      document.documentElement.classList.toggle("dark", dark);
      document.documentElement.dataset.theme = settings.theme;
      document.documentElement.dataset.density = settings.density;
    });
  });

  it("applies the current settings immediately on subscribe", () => {
    cleanup = startThemeSubscription();
    expect(applied).toHaveLength(1);
    expect(applied[0]).toEqual(DEFAULT_SETTINGS);
  });

  it("reapplies when settings change", () => {
    cleanup = startThemeSubscription();
    applied.length = 0;
    useSettingsStore.getState().updateSettings({ theme: "dark" });
    expect(applied).toHaveLength(1);
    expect(applied[0]!.theme).toBe("dark");
  });

  it("stops applying after cleanup", () => {
    cleanup = startThemeSubscription();
    cleanup();
    cleanup = null;
    applied.length = 0;
    useSettingsStore.getState().updateSettings({ theme: "dark" });
    expect(applied).toEqual([]);
  });
});
