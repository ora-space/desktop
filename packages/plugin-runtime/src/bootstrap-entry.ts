import { runBootstrap } from "./bootstrap/main.js";

void runBootstrap().catch((error: unknown) => {
  const message = error instanceof Error ? error.message : "unknown bootstrap failure";
  process.stderr.write(`[ora-plugin-bootstrap:fatal] ${message.slice(0, 8192)}\n`);
  process.exitCode = 1;
});
