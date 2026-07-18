import { BootstrapSession, installStdoutGuard, type BootstrapOptions } from "./session.js";

/** Runs the private bootstrap against process stdio or an injected fake Host transport. */
export async function runBootstrap(options: BootstrapOptions = {}): Promise<void> {
  const stdout = process.stdout;
  const stderr = process.stderr;
  const session = new BootstrapSession({ stdin: process.stdin, stdout, stderr }, options);
  installStdoutGuard(stderr);
  await session.run();
}
