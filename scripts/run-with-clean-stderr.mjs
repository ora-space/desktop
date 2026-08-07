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
    process.stderr.write(
      "test command wrote to stderr; treating the run as failed\n" +
        "----- captured stderr begin -----\n" +
        formatCapturedStderr(result.stderrText) +
        "----- captured stderr end -----\n",
    );
    process.exitCode = 1;
  }
}

/** Renders captured stderr for the failure summary (shows controls/whitespace clearly). */
function formatCapturedStderr(text) {
  if (text.length === 0) {
    return "(empty)\n";
  }
  return `${text}${text.endsWith("\n") ? "" : "\n"}(json: ${JSON.stringify(text)})\n`;
}

/**
 * Runs one trusted package command while preserving output and recording stderr use.
 *
 * Forces CI mode so Vitest picks the non-interactive reporter. The default TTY
 * reporter clears the screen and can hide the same stderr this gate fails on.
 */
function runCommand(command) {
  return new Promise((resolve) => {
    const child = spawn(command, {
      shell: true,
      stdio: ["inherit", "inherit", "pipe"],
      windowsHide: true,
      env: {
        ...process.env,
        CI: process.env.CI && process.env.CI !== "" ? process.env.CI : "1",
      },
    });
    let wroteToStderr = false;
    let stderrText = "";

    child.stderr.on("data", (chunk) => {
      wroteToStderr ||= chunk.length > 0;
      stderrText += chunk.toString("utf8");
      process.stderr.write(chunk);
    });
    child.on("error", (error) => {
      process.stderr.write(`failed to start test command: ${error.message}\n`);
      resolve({ exitCode: 1, wroteToStderr: true, stderrText });
    });
    child.on("close", (exitCode, signal) => {
      if (signal !== null) {
        process.stderr.write(`test command terminated by signal ${signal}\n`);
      }
      resolve({
        exitCode: exitCode ?? 1,
        wroteToStderr: wroteToStderr || signal !== null,
        stderrText,
      });
    });
  });
}
