import { fileURLToPath } from "node:url";
import { existsSync } from "node:fs";
import { chmod, mkdir, mkdtemp, readdir, rename, rm } from "node:fs/promises";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import path from "node:path";
import process from "node:process";
import { checkToolchain, readDenoVersion } from "./check-toolchain.ts";
import {
  readSidecarChecksums,
  verifySidecarHash,
} from "./sidecar-integrity.ts";

export interface SetupOptions {
  root?: string;
  args?: string[];
  environment?: NodeJS.ProcessEnv;
  platform?: string;
  arch?: string;
  exec?: (command: string, args: string[]) => Promise<unknown>;
}
interface BinaryAsset {
  name: "deno" | "ripgrep";
  version: string;
  asset: string;
  archiveExtension: "zip" | "tar.gz";
  archiveExecutableName: string;
  executableName: string;
}

/** Installs only requested sidecars, with injectable commands for offline filesystem tests. */
export async function setupBinaries(options: SetupOptions = {}): Promise<void> {
  const environment = options.environment ?? process.env;
  const platform = options.platform ?? process.platform;
  const arch = options.arch ?? process.arch;
  const args = options.args ?? Deno.args;
  const execFileAsync = options.exec ?? promisify(execFile);
  const ripgrepVersion = environment.RG_VERSION ?? "15.2.0";
  const repositoryRoot =
    options.root ?? fileURLToPath(new URL("..", import.meta.url));
  const binaryDirectory = path.join(
    repositoryRoot,
    "apps",
    "desktop",
    "src-tauri",
    "binaries",
  );
  const requestedBinaries = args.filter((argument) => argument !== "--force");
  const supportedBinaries = new Set(["deno", "rg"]);
  for (const binary of requestedBinaries) {
    if (!supportedBinaries.has(binary)) {
      throw new Error(
        `Unsupported binary '${binary}'. Expected one of: deno, rg.`,
      );
    }
  }
  const binariesToInstall = new Set(
    requestedBinaries.length > 0 ? requestedBinaries : supportedBinaries,
  );
  const denoVersion = binariesToInstall.has("deno")
    ? await readDenoVersion(repositoryRoot)
    : undefined;
  const requestedDenoVersion = environment.DENO_VERSION?.replace(/^v/, "");
  if (
    denoVersion &&
    requestedDenoVersion !== undefined &&
    requestedDenoVersion !== denoVersion
  ) {
    throw new Error(
      "DENO_VERSION must match .deno-version; update the shared pin to change the runtime.",
    );
  }

  /** Resolves the configured build target or the current development machine target. */
  function targetTriple(): string {
    const configuredTriple =
      environment.TARGET_TRIPLE ??
      environment.TAURI_ENV_TARGET_TRIPLE ??
      environment.RUST_TARGET;
    if (configuredTriple) return configuredTriple;

    if (platform === "darwin") {
      return arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
    }
    if (platform === "win32") {
      return "x86_64-pc-windows-msvc";
    }
    if (platform === "linux") {
      return "x86_64-unknown-linux-gnu";
    }
    throw new Error(`Unsupported platform: ${platform}-${arch}`);
  }

  /** Downloads a release archive through the configured proxy, if one is present. */
  async function download(url: string, destination: string): Promise<void> {
    const proxy = [
      environment.HTTPS_PROXY,
      environment.https_proxy,
      environment.HTTP_PROXY,
      environment.http_proxy,
      environment.ALL_PROXY,
      environment.all_proxy,
      environment.PROXY,
      environment.proxy,
    ].find(Boolean);
    const args = [
      "--fail",
      "--location",
      "--retry",
      "3",
      "--output",
      destination,
    ];
    if (proxy) {
      args.push("--proxy", proxy);
      console.log("Using configured proxy for the sidecar downloads.");
    }
    await execFileAsync("curl", [...args, url]);
  }

  /** Finds an extracted executable without depending on the archive's directory layout. */
  async function findExtractedBinary(
    directory: string,
    fileName: string,
  ): Promise<string | undefined> {
    const entries = await readdir(directory, { withFileTypes: true });
    for (const entry of entries) {
      const entryPath = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        const nestedPath = await findExtractedBinary(entryPath, fileName);
        if (nestedPath) return nestedPath;
      } else if (entry.isFile() && entry.name === fileName) {
        return entryPath;
      }
    }
    return undefined;
  }

  /** Extracts one downloaded archive and renames its executable to Tauri's sidecar convention. */
  async function installBinary({
    name,
    version,
    asset,
    archiveExtension,
    archiveExecutableName,
    executableName,
  }: BinaryAsset): Promise<void> {
    const isWindowsTarget = triple.endsWith("-windows-msvc");
    const destination = path.join(binaryDirectory, executableName);
    const project = name === "deno" ? "denoland/deno" : "BurntSushi/ripgrep";
    const releaseVersion = version.replace(/^v/, "");
    const releaseTag = name === "deno" ? `v${releaseVersion}` : releaseVersion;
    const url = `https://github.com/${project}/releases/download/${releaseTag}/${asset}`;
    const checksums = await readSidecarChecksums(repositoryRoot, url);

    /** Hash identity also validates foreign targets that cannot execute on the build host. */
    async function verifyExecutable(file: string): Promise<void> {
      await verifySidecarHash(file, checksums.executableSha256);
      const hostTriple =
        platform === "darwin"
          ? `${arch === "arm64" ? "aarch64" : "x86_64"}-apple-darwin`
          : platform === "win32"
            ? "x86_64-pc-windows-msvc"
            : "x86_64-unknown-linux-gnu";
      if (triple !== hostTriple) return;
      const result = await execFileAsync(file, ["--version"]);
      const reported =
        typeof result === "object" &&
        result !== null &&
        "stdout" in result &&
        typeof result.stdout === "string"
          ? result.stdout.match(/^(deno|ripgrep) (\S+)(?:\s|$)/)
          : null;
      if (reported?.[1] !== name || reported?.[2] !== releaseVersion) {
        throw new Error(`Expected ${name} ${releaseVersion} from ${file}.`);
      }
    }

    if (!args.includes("--force") && existsSync(destination)) {
      try {
        await verifyExecutable(destination);
        console.log(`${name} sidecar is verified for ${triple}.`);
        return;
      } catch {
        // Replace stale, corrupt, or unexecutable cache entries only after a verified download.
      }
    }
    // Same-filesystem staging keeps publication atomic. Concurrent installers own
    // separate archives and extraction trees and can only publish the same pinned bytes.
    const temporaryDirectory = await mkdtemp(
      path.join(binaryDirectory, `.install-${name}-`),
    );
    const archivePath = path.join(
      temporaryDirectory,
      `archive.${archiveExtension}`,
    );
    const extractDirectory = path.join(temporaryDirectory, "extracted");
    console.log(`Downloading ${name} ${version} for ${triple}...`);
    try {
      await download(url, archivePath);
      await verifySidecarHash(archivePath, checksums.archiveSha256);
      await mkdir(extractDirectory);
      if (platform === "win32" && archiveExtension === "zip") {
        await execFileAsync("powershell", [
          "-NoProfile",
          "-ExecutionPolicy",
          "Bypass",
          "-Command",
          `Expand-Archive -LiteralPath '${archivePath.replaceAll("'", "''")}' -DestinationPath '${extractDirectory.replaceAll("'", "''")}' -Force`,
        ]);
      } else if (archiveExtension === "zip") {
        await execFileAsync("unzip", [
          "-q",
          archivePath,
          "-d",
          extractDirectory,
        ]);
      } else {
        await execFileAsync("tar", [
          "-xzf",
          archivePath,
          "--strip-components=1",
          "-C",
          extractDirectory,
        ]);
      }
      const extractedExecutableName = isWindowsTarget
        ? `${archiveExecutableName}.exe`
        : archiveExecutableName;
      const extractedExecutable = await findExtractedBinary(
        extractDirectory,
        extractedExecutableName,
      );
      if (!extractedExecutable) {
        throw new Error(
          `Archive for ${name} does not contain ${extractedExecutableName}.`,
        );
      }
      if (!isWindowsTarget) {
        await chmod(extractedExecutable, 0o755);
      }
      await verifyExecutable(extractedExecutable);
      await rename(extractedExecutable, destination);
    } finally {
      await rm(temporaryDirectory, { force: true, recursive: true });
    }
  }

  const triple = targetTriple();
  const isWindows = triple.endsWith("-windows-msvc");
  const denoAsset = `deno-${triple}.zip`;
  // The Linux ripgrep release ships a static musl binary, while Tauri still uses
  // the host GNU triple in the sidecar filename and environment variables.
  const ripgrepAssetTriple =
    triple === "x86_64-unknown-linux-gnu"
      ? "x86_64-unknown-linux-musl"
      : triple;
  const ripgrepAsset = `ripgrep-${ripgrepVersion}-${ripgrepAssetTriple}.${isWindows ? "zip" : "tar.gz"}`;

  await mkdir(binaryDirectory, { recursive: true });
  if (denoVersion) {
    await installBinary({
      name: "deno",
      version: denoVersion,
      asset: denoAsset,
      archiveExtension: "zip",
      archiveExecutableName: "deno",
      executableName: `deno-${triple}${isWindows ? ".exe" : ""}`,
    });
  }
  if (binariesToInstall.has("rg")) {
    await installBinary({
      name: "ripgrep",
      version: ripgrepVersion,
      asset: ripgrepAsset,
      archiveExtension: isWindows ? "zip" : "tar.gz",
      archiveExecutableName: "rg",
      executableName: `rg-${triple}${isWindows ? ".exe" : ""}`,
    });
  }
}
if (import.meta.main) {
  await checkToolchain();
  await setupBinaries();
}
