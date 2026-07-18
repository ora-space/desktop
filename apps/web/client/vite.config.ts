import { defineConfig, loadEnv } from 'vite'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import * as path from 'node:path'

// https://vite.dev/config/
export default defineConfig(({ command, mode }) => {
  const configuredTransport = loadEnv(mode, __dirname, "").VITE_ORA_CONTRACT_TRANSPORT
  const transport = command === "build" ? "fetch" : configuredTransport
  if (transport !== "fetch" && transport !== "mock") {
    throw new Error("VITE_ORA_CONTRACT_TRANSPORT must be set to either `fetch` or `mock` in development")
  }

  return {
    plugins: [react(), tailwindcss()],
    resolve: {
      alias: [
        {
          find: "@/contracts-runtime",
          replacement: path.resolve(
            __dirname,
            transport === "mock" ? "./src/contracts-runtime.mock.ts" : "./src/contracts-runtime.ts",
          ),
        },
        { find: "@", replacement: path.resolve(__dirname, "./src") },
        { find: /^@ora\/app-shell$/, replacement: path.resolve(__dirname, "../../../packages/app-shell/src/index.ts") },
        { find: /^@ora\/contracts$/, replacement: path.resolve(__dirname, "../../../packages/contracts/src/index.ts") },
        { find: /^@ora\/ui$/, replacement: path.resolve(__dirname, "../../../packages/ui/src/index.ts") },
      ],
    },
    server: {
      host: "0.0.0.0",
      proxy: {
        "/api": {
          target: "http://localhost:32578",
          changeOrigin: true,
        },
      },
    },
  }
})
