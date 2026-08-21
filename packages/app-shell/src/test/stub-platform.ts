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
      open: async (definition, target) => ({
        instance: 0,
        pluginId: definition.pluginId,
        surfaceId: definition.surfaceId,
        title: definition.surfaceId,
        target,
        state: "open",
      }),
      close: async () => undefined,
      setBounds: async () => undefined,
      setVisible: async () => undefined,
      popout: async () => undefined,
      dock: async () => undefined,
      reload: async () => undefined,
      onEvent: async () => () => undefined,
    },
    selectPath: async () => null,
    saveTextFile: async () => false,
    openExternalUrl: async () => undefined,
  };
}
