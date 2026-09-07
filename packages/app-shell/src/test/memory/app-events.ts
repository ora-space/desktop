import type { TestHandlers } from "../contracts-transport";

/** Registers only the app-events operations explicitly requested by a fixture. */
export function appEventHandlers() {
  return {
    watchAppEvents: async function* (_request, options) {
      yield { type: "ready" as const };
      await new Promise<void>((resolve) => {
        const signal = options?.signal;
        if (signal === undefined) return;
        if (signal.aborted) {
          resolve();
          return;
        }
        signal.addEventListener("abort", () => resolve(), { once: true });
      });
    },
  } satisfies TestHandlers;
}
