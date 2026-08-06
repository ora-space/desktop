import { spawn } from "node:child_process";

const [command, ...unexpectedArguments] = process.argv.slice(2);

if (command === undefined || unexpectedArguments.length > 0) {
  process.stderr.write("usage: node scripts/run-with-clean-stderr.mjs \"<command>\"\n");
  process.exitCode = 2;
} else {
  const result = await runCommand(command);
  if (result.exitCode !== 0) {
    process.exitCode = result.exitCode;
  } else if (result.wroteToStderr) {
    process.stderr.write("test command wrote to stderr; treating the run as failed\n");
    process.exitCode = 1;
  }
}

/** Runs one trusted package command while preserving output and recording stderr use. */
function runCommand(command) {
  return new Promise((resolve) => {
    const child = spawn(command, {
      shell: true,
      stdio: ["inherit", "inherit", "pipe"],
      windowsHide: true,
    });
    let wroteToStderr = false;

    child.stderr.on("data", (chunk) => {
      wroteToStderr ||= chunk.length > 0;
      process.stderr.write(chunk);
    });
    child.on("error", (error) => {
      process.stderr.write(`failed to start test command: ${error.message}\n`);
      resolve({ exitCode: 1, wroteToStderr: true });
    });
    child.on("close", (exitCode, signal) => {
      if (signal !== null) {
        process.stderr.write(`test command terminated by signal ${signal}\n`);
      }
      resolve({
        exitCode: exitCode ?? 1,
        wroteToStderr: wroteToStderr || signal !== null,
      });
    });
  });
}
