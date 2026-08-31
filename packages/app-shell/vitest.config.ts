import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "node:path";

export default defineConfig({
  plugins: [react()],
  // Keep progress on stdout; the default TTY reporter clears the screen and
  // can hide stderr that run-with-clean-stderr is gating on.
  clearScreen: false,
  resolve: {
    alias: {
      "@ora/editor/composer": path.resolve(
        __dirname,
        "../editor/src/composer/index.ts",
      ),
      "@ora/editor": path.resolve(__dirname, "../editor/src/index.ts"),
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    // Set ORA_VITEST_MAX_WORKERS only on machines that need a lower memory peak;
    // leaving it unset preserves Vitest's existing defaults for everyone else.
    maxWorkers: process.env.ORA_VITEST_MAX_WORKERS
      ? Number.parseInt(process.env.ORA_VITEST_MAX_WORKERS, 10)
      : undefined,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    css: false,
    // Full-shell tests (AppShell + workflow editor) take well under a second
    // in isolation but exceed the 5s default when `task test` loads the machine
    // with parallel frontend and Rust workers, which flakes them spuriously.
    testTimeout: 15_000,
  },
});
