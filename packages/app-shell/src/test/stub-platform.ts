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
    skillMarketplace: {
      open: async () => undefined,
      onStatus: async () => () => undefined,
    },
    selectPath: async () => null,
    saveTextFile: async () => false,
  };
}
