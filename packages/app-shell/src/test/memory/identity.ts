import type { TestHandlers } from "../contracts-transport";

/** Registers only the identity operations explicitly requested by a fixture. */
export function identityHandlers() {
  return {
    getGitIdentity: async () => ({
      name: "Test User",
      email: "test@ora.local",
    }),
  } satisfies TestHandlers;
}
