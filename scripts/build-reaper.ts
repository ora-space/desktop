import { fileURLToPath } from "node:url";
import { copyFile, mkdir } from "node:fs/promises";
import { spawn } from "node:child_process";
import path from "node:path";
import process from "node:process";

export interface ReaperOptions {
  root?: string;
  args?: string[];
  environment?: NodeJS.ProcessEnv;
  run?: (command: string, args: string[]) => Promise<void>;
  hostTarget?: () => Promise<string>;
}

/** Builds and installs the target-specific reaper; command seams keep tests out of Cargo. */
export async function buildReaper(options: ReaperOptions = {}): Promise<void> {
  const repositoryRoot =
    options.root ?? fileURLToPath(new URL("..", import.meta.url));
  const environment = options.environment ?? process.env;
  const release = (options.args ?? Deno.args).includes("--release");
  const configuredTarget =
    environment.TARGET_TRIPLE ??
    environment.TAURI_ENV_TARGET_TRIPLE ??
    environment.RUST_TARGET;

  /** Runs one build command with its output attached to the invoking task. */
  function run(command: string, args: string[]): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      const child = spawn(command, args, {
        cwd: repositoryRoot,
        stdio: "inherit",
        env: environment,
      });
      child.once("error", reject);
      child.once("exit", (code, signal) => {
        if (code === 0) {
          resolve();
        } else {
          reject(
            new Error(
              `${command} exited with ${code ?? `signal ${signal ?? "unknown"}`}`,
            ),
          );
        }
      });
    });
  }

  /** Reads rustc's canonical host triple when no cross-compilation target was requested. */
  async function hostTargetTriple(): Promise<string> {
    let output = "";
    await new Promise<void>((resolve, reject) => {
      const child = spawn("rustc", ["-vV"], {
        cwd: repositoryRoot,
        env: environment,
      });
      child.stdout.on("data", (chunk) => {
        output += chunk.toString();
      });
      child.once("error", reject);
      child.once("exit", (code) => {
        if (code === 0) resolve();
        else reject(new Error(`rustc -vV exited with ${code}`));
      });
    });
    const host = output.match(/^host:\s*(.+)$/m)?.[1];
    if (!host) throw new Error("rustc -vV did not report a host target");
    return host.trim();
  }

  const target =
    configuredTarget ?? (await (options.hostTarget ?? hostTargetTriple)());
  const cargoArgs = ["build", "--package", "ora-reaper"];
  if (release) cargoArgs.push("--release");
  if (configuredTarget) cargoArgs.push("--target", configuredTarget);
  await (options.run ?? run)("cargo", cargoArgs);

  const profile = release ? "release" : "debug";
  const executableSuffix = target.includes("windows") ? ".exe" : "";
  const targetDirectory = environment.CARGO_TARGET_DIR
    ? path.resolve(repositoryRoot, environment.CARGO_TARGET_DIR)
    : path.join(repositoryRoot, "target");
  const source = configuredTarget
    ? path.join(
        targetDirectory,
        target,
        profile,
        `ora-reaper${executableSuffix}`,
      )
    : path.join(targetDirectory, profile, `ora-reaper${executableSuffix}`);
  const binaryDirectory = path.join(
    repositoryRoot,
    "apps",
    "desktop",
    "src-tauri",
    "binaries",
  );
  const destination = path.join(
    binaryDirectory,
    `ora-reaper-${target}${executableSuffix}`,
  );
  await mkdir(binaryDirectory, { recursive: true });
  await copyFile(source, destination);
  console.log(`Installed ora-reaper sidecar at ${destination}`);
}
if (import.meta.main) await buildReaper();
