import { spawn } from "node:child_process";
import {
  constants,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  stat,
} from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { randomBytes } from "node:crypto";

const MAX_TOOL_OUTPUT_BYTES = 1024 * 1024;
const OPTIONAL_ROOT_FILE = /^(?:README|LICENSE)(?:\.[^/\\]+)?$/i;

export interface PackAgentPluginOptions {
  readonly sourceRoot: string;
  readonly entry: string;
  readonly outputRoot: string;
  readonly bunExecutable?: string;
}

export interface PackAgentPluginResult {
  readonly artifactRoot: string;
  readonly metafilePath: string;
}

export interface ValidateArtifactOptions {
  readonly artifactRoot: string;
  readonly metafilePath?: string;
  readonly bunExecutable?: string;
}

type BuildImport = {
  readonly path: string;
  readonly kind: string;
  readonly external?: boolean;
};

type BuildMetafile = {
  readonly inputs: Readonly<Record<string, { readonly imports?: readonly BuildImport[] }>>;
  readonly outputs: Readonly<Record<string, { readonly imports?: readonly BuildImport[] }>>;
};

type ParsedImport = {
  readonly path: string;
  readonly kind: string;
};

/** Produces one no-link materialized Agent artifact with a sibling build metafile. */
export async function packAgentPlugin(
  options: PackAgentPluginOptions,
): Promise<PackAgentPluginResult> {
  const sourceRoot = await realpath(resolve(options.sourceRoot));
  const entry = resolve(sourceRoot, options.entry);
  if (!isWithin(sourceRoot, entry)) {
    throw new Error("entry must remain inside the source root");
  }
  const entryStats = await stat(entry);
  if (!entryStats.isFile()) {
    throw new Error("entry must be a regular file");
  }
  const outputRoot = resolve(options.outputRoot);
  if (isWithin(sourceRoot, outputRoot)) {
    throw new Error("pack output must be outside the source tree");
  }
  await requireMissing(outputRoot);
  const metafilePath = `${outputRoot}.metafile.json`;
  await requireMissing(metafilePath);
  await mkdir(dirname(outputRoot), { recursive: true });
  const stagingRoot = await mkdtemp(join(dirname(outputRoot), ".ora-pack-"));
  const artifactRoot = join(stagingRoot, "artifact");
  const stagingMetafile = join(stagingRoot, "build-metafile.json");
  const bunExecutable = await resolvePinnedBun(options.bunExecutable);
  const emptyBunfig = preparedEmptyBunfig(bunExecutable);

  try {
    await mkdir(join(artifactRoot, "dist"), { recursive: true });
    await copyPackageFiles(sourceRoot, artifactRoot);
    const builder = fileURLToPath(new URL("./builder.js", import.meta.url));
    await runBounded(bunExecutable, [
      `--config=${emptyBunfig}`,
      "--no-env-file",
      "--no-macros",
      builder,
      entry,
      join(artifactRoot, "dist", "index.js"),
      stagingMetafile,
    ], sourceRoot);
    await validateMaterializedArtifact({
      artifactRoot,
      metafilePath: stagingMetafile,
      bunExecutable,
    });
    await rename(artifactRoot, outputRoot);
    try {
      await rename(stagingMetafile, metafilePath);
    } catch (error) {
      await rm(outputRoot, { recursive: true, force: true });
      throw error;
    }
    return { artifactRoot: outputRoot, metafilePath };
  } finally {
    await rm(stagingRoot, { recursive: true, force: true });
  }
}

/** Validates the artifact allowlist, build graph, and parsed surviving imports without execution. */
export async function validateMaterializedArtifact(
  options: ValidateArtifactOptions,
): Promise<void> {
  const artifactRoot = await realpath(resolve(options.artifactRoot));
  await auditArtifactTree(artifactRoot, artifactRoot);
  const packageJson = JSON.parse(await readFile(join(artifactRoot, "package.json"), "utf8")) as {
    readonly type?: unknown;
    readonly ora?: { readonly main?: unknown };
  };
  if (packageJson.type !== "module" || packageJson.ora?.main !== "dist/index.js") {
    throw new Error("artifact package.json must declare type=module and ora.main=dist/index.js");
  }
  const entry = join(artifactRoot, "dist", "index.js");
  const bunExecutable = await resolvePinnedBun(options.bunExecutable);
  const emptyBunfig = preparedEmptyBunfig(bunExecutable);
  const scanner = fileURLToPath(new URL("./scanner.js", import.meta.url));
  const scanOutput = await runBounded(
    bunExecutable,
    [`--config=${emptyBunfig}`, "--no-env-file", "--no-macros", scanner, entry],
    artifactRoot,
  );
  const imports = JSON.parse(scanOutput.stdout) as ParsedImport[];
  for (const imported of imports) {
    if (!isRuntimeBuiltin(imported.path)) {
      throw new Error(`artifact retains ${imported.kind} import: ${imported.path || "<dynamic>"}`);
    }
  }
  if (options.metafilePath) {
    validateMetafile(
      JSON.parse(await readFile(options.metafilePath, "utf8")) as BuildMetafile,
    );
  }
}

/** Resolves only the explicitly configured or repository-prepared Bun executable. */
export async function resolvePinnedBun(configured?: string): Promise<string> {
  const executable = configured ?? process.env.ORA_PINNED_BUN ?? defaultPreparedBun();
  if (!isAbsolute(executable)) {
    throw new Error("pinned Bun path must be absolute");
  }
  const executableStats = await stat(executable);
  if (!executableStats.isFile()) {
    throw new Error("pinned Bun executable is unavailable");
  }
  return executable;
}

function defaultPreparedBun(): string {
  const moduleDirectory = dirname(fileURLToPath(import.meta.url));
  return resolve(moduleDirectory, "..", "..", "..", "..", "runtime-assets", "prepared", "bun.exe");
}

function preparedEmptyBunfig(bunExecutable: string): string {
  return join(dirname(bunExecutable), "empty-bunfig.toml");
}

async function copyPackageFiles(sourceRoot: string, artifactRoot: string): Promise<void> {
  const packagePath = join(sourceRoot, "package.json");
  await requireRegularUnlinkedFile(packagePath);
  await copyFile(packagePath, join(artifactRoot, "package.json"), constants.COPYFILE_EXCL);
  for (const entry of await readdir(sourceRoot, { withFileTypes: true })) {
    if (!OPTIONAL_ROOT_FILE.test(entry.name)) {
      continue;
    }
    const source = join(sourceRoot, entry.name);
    await requireRegularUnlinkedFile(source);
    await copyFile(source, join(artifactRoot, entry.name), constants.COPYFILE_EXCL);
  }
}

async function auditArtifactTree(root: string, directory: string): Promise<void> {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    const metadata = await lstat(path);
    if (metadata.isSymbolicLink() || metadata.nlink !== 1) {
      throw new Error("artifact contains a link or multiply-linked object");
    }
    const relativePath = relative(root, path).split(sep).join("/");
    if (entry.isDirectory()) {
      if (relativePath !== "dist") {
        throw new Error(`artifact directory is not allowed: ${relativePath}`);
      }
      await auditArtifactTree(root, path);
      continue;
    }
    if (!entry.isFile()) {
      throw new Error(`artifact object is not a regular file: ${relativePath}`);
    }
    const allowed = relativePath === "package.json"
      || relativePath === "dist/index.js"
      || (!relativePath.includes("/") && OPTIONAL_ROOT_FILE.test(relativePath));
    if (!allowed || relativePath.toLowerCase().endsWith(".node") || relativePath.includes("node_modules")) {
      throw new Error(`artifact file is not allowed: ${relativePath}`);
    }
  }
}

function validateMetafile(metafile: BuildMetafile): void {
  if (!metafile || typeof metafile !== "object") {
    throw new Error("Bun metafile is invalid");
  }
  const imports = [
    ...Object.values(metafile.inputs).flatMap((input) => input.imports ?? []),
    ...Object.values(metafile.outputs).flatMap((output) => output.imports ?? []),
  ];
  for (const imported of imports) {
    if (imported.external && !isRuntimeBuiltin(imported.path)) {
      throw new Error(`metafile retains external import: ${imported.path}`);
    }
    if (imported.path.toLowerCase().endsWith(".node")) {
      throw new Error(`metafile references a native addon: ${imported.path}`);
    }
  }
}

function isRuntimeBuiltin(specifier: string): boolean {
  return specifier === "bun" || specifier.startsWith("bun:") || specifier.startsWith("node:");
}

function isWithin(root: string, candidate: string): boolean {
  const difference = relative(root, candidate);
  return difference === "" || (!difference.startsWith(`..${sep}`) && difference !== ".." && !isAbsolute(difference));
}

async function requireMissing(path: string): Promise<void> {
  try {
    await lstat(path);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      return;
    }
    throw error;
  }
  throw new Error(`pack destination already exists: ${path}`);
}

async function requireRegularUnlinkedFile(path: string): Promise<void> {
  const metadata = await lstat(path);
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.nlink !== 1) {
    throw new Error(`source package file is unsafe: ${path}`);
  }
}

async function runBounded(
  program: string,
  args: readonly string[],
  cwd: string,
): Promise<{ stdout: string; stderr: string }> {
  return new Promise((resolveRun, rejectRun) => {
    const child = spawn(program, args, {
      cwd,
      env: process.env,
      shell: false,
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    let outputBytes = 0;
    let overflow = false;
    const append = (target: Buffer[], chunk: Buffer): void => {
      outputBytes += chunk.length;
      if (outputBytes > MAX_TOOL_OUTPUT_BYTES) {
        overflow = true;
        child.kill();
        return;
      }
      target.push(chunk);
    };
    child.stdout.on("data", (chunk: Buffer) => append(stdout, chunk));
    child.stderr.on("data", (chunk: Buffer) => append(stderr, chunk));
    child.once("error", rejectRun);
    child.once("close", (code) => {
      const result = {
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      };
      if (overflow) {
        rejectRun(new Error("Bun tool output exceeded its bound"));
      } else if (code !== 0) {
        rejectRun(new Error(`pinned Bun failed (${code}): ${result.stderr.slice(0, 4096)}`));
      } else {
        resolveRun(result);
      }
    });
  });
}

/** Creates a collision-resistant path suffix for callers that need independent pack destinations. */
export function randomPackSuffix(): string {
  return randomBytes(16).toString("hex");
}
