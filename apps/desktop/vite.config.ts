import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
import * as path from "node:path";

/** Reads the workspace version that Cargo.toml owns for the whole Rust workspace. */
function readWorkspaceVersion(): string {
  const cargoToml = readFileSync(
    path.resolve(__dirname, "../../Cargo.toml"),
    "utf8",
  );
  const match = cargoToml.match(
    /^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m,
  );
  if (match === null) {
    throw new Error("workspace.package version missing in Cargo.toml");
  }
  return match[1];
}

// https://vite.dev/config/
export default defineConfig({
  define: {
    __ORA_APP_VERSION__: JSON.stringify(readWorkspaceVersion()),
  },
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: [
      { find: "@", replacement: path.resolve(__dirname, "./web") },
      {
        find: /^@ora\/app-shell$/,
        replacement: path.resolve(
          __dirname,
          "../../packages/app-shell/src/index.ts",
        ),
      },
      {
        find: /^@ora\/chat$/,
        replacement: path.resolve(
          __dirname,
          "../../packages/chat/src/index.ts",
        ),
      },
      {
        find: /^@ora\/contracts$/,
        replacement: path.resolve(
          __dirname,
          "../../packages/contracts/src/index.ts",
        ),
      },
      {
        find: /^@ora\/editor\/composer$/,
        replacement: path.resolve(
          __dirname,
          "../../packages/editor/src/composer/index.ts",
        ),
      },
      {
        find: /^@ora\/editor$/,
        replacement: path.resolve(
          __dirname,
          "../../packages/editor/src/index.ts",
        ),
      },
      {
        find: /^@ora\/ui$/,
        replacement: path.resolve(__dirname, "../../packages/ui/src/index.ts"),
      },
      {
        find: /^@ora\/workflow-mock$/,
        replacement: path.resolve(
          __dirname,
          "../../packages/workflow-mock/src/index.ts",
        ),
      },
      {
        find: /^@ora\/workflow-runtime$/,
        replacement: path.resolve(
          __dirname,
          "../../packages/workflow-runtime/src/index.ts",
        ),
      },
      {
        find: /^@ora\/workflow-runtime\/memory$/,
        replacement: path.resolve(
          __dirname,
          "../../packages/workflow-runtime/src/memory.ts",
        ),
      },
    ],
  },
  server: {
    // Bind the loopback address Tauri actually loads. Vite's default 5173 is often
    // already taken on Windows by the IDE's localhost port-forward, which binds
    // 127.0.0.1 more specifically than a 0.0.0.0 listener and returns empty HTTP
    // replies (Chrome ERR_EMPTY_RESPONSE). Port 1420 is the Tauri template default.
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    watch: {
      // The Tauri Rust build target dir is constantly rewritten by cargo; watching
      // it on Windows raises EBUSY and crashes the Vite dev watcher.
      ignored: [path.resolve(__dirname, "../../target")],
    },
  },
});
