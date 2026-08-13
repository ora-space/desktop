import { defineConfig } from 'vite'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import * as path from 'node:path'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: [
      { find: "@", replacement: path.resolve(__dirname, "./web") },
      { find: /^@ora\/app-shell$/, replacement: path.resolve(__dirname, "../../packages/app-shell/src/index.ts") },
      { find: /^@ora\/chat$/, replacement: path.resolve(__dirname, "../../packages/chat/src/index.ts") },
        { find: /^@ora\/contracts$/, replacement: path.resolve(__dirname, "../../packages/contracts/src/index.ts") },
      { find: /^@ora\/ui$/, replacement: path.resolve(__dirname, "../../packages/ui/src/index.ts") },
      { find: /^@ora\/workflow-mock$/, replacement: path.resolve(__dirname, "../../packages/workflow-mock/src/index.ts") },
      { find: /^@ora\/workflow-runtime$/, replacement: path.resolve(__dirname, "../../packages/workflow-runtime/src/index.ts") },
      { find: /^@ora\/workflow-runtime\/memory$/, replacement: path.resolve(__dirname, "../../packages/workflow-runtime/src/memory.ts") },
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
      ignored: ["**/src-tauri/target/**"],
    },
    proxy: {
      "/api": {
        target: "http://localhost:21688",
        changeOrigin: true,
      },
    },
  },
})
