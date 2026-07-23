import type { PlatformAdapter } from "@ora/platform";

/**
 * A no-op platform adapter for component tests.
 *
 * Any component that reaches the title bar now reads `usePlatform()`, so tests
 * that render the workspace shell need a provider. This reports every capability
 * as absent, which keeps the custom window controls unrendered and matches how
 * the Web host behaves.
 */
export function createStubPlatform(): PlatformAdapter {
  return {
    worktreeStorage: { kind: "unsupported" },
    windowControls: { kind: "none" },
    selectPath: async () => null,
  };
}
