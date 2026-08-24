import type { PlatformAdapter } from "../platform";

/**
 * A no-op platform adapter for component tests.
 *
 * Any component that reaches the title bar now reads `usePlatform()`, so tests
 * that render the workspace shell need a provider. Native capabilities are
 * harmless test doubles; tests that exercise them inject a recording adapter.
 */
export function createStubPlatform(): PlatformAdapter {
  return {
    worktreeStorage: {
      getRoot: async () => "",
      setRoot: async () => undefined,
    },
    windowControls: { kind: "none" },
    locationActions: {
      resolveTaskCwd: async () => "",
      open: async () => undefined,
    },
    surfaces: {
      capabilities: async () => ({ embedded: false }),
      list: async () => [],
      open: async (target, mount) => ({
        instance: 0,
        pluginId: target.pluginId,
        kind: "webview" as const,
        title: target.pluginId,
        target: mount,
        state: "open" as const,
      }),
      close: async () => undefined,
      resolveDownload: async () => ({
        action: "save_as",
        importSessionId: null,
      }),
      discardDownload: async () => undefined,
      setBounds: async () => undefined,
      setVisible: async () => undefined,
      popout: async () => undefined,
      dock: async () => undefined,
      reload: async () => undefined,
      onEvent: async () => () => undefined,
    },
    selectPath: async () => null,
    selectSavePath: async () => null,
    saveTextFile: async () => false,
    openExternalUrl: async () => undefined,
  };
}
