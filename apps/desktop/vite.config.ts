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
    ],
  },
  server: {
    host: "0.0.0.0",
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
