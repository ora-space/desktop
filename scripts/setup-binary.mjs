import { existsSync } from "node:fs";
import { chmod, mkdir, readdir, rename, rm } from "node:fs/promises";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import path from "node:path";
import process from "node:process";

const execFileAsync = promisify(execFile);
const denoVersion = process.env.DENO_VERSION ?? "v2.9.5";
const ripgrepVersion = process.env.RG_VERSION ?? "15.2.0";
const repositoryRoot = path.resolve(import.meta.dirname, "..");
const binaryDirectory = path.join(
  repositoryRoot,
  "apps",
  "desktop",
  "src-tauri",
  "binaries",
);
const requestedBinaries = process.argv
  .slice(2)
  .filter((argument) => argument !== "--force");
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

/** Resolves the configured build target or the current development machine target. */
function targetTriple() {
  const configuredTriple =
    process.env.TARGET_TRIPLE ??
    process.env.TAURI_ENV_TARGET_TRIPLE ??
    process.env.RUST_TARGET;
  if (configuredTriple) return configuredTriple;

  if (process.platform === "darwin") {
    return process.arch === "arm64"
      ? "aarch64-apple-darwin"
      : "x86_64-apple-darwin";
  }
  if (process.platform === "win32") {
    return "x86_64-pc-windows-msvc";
  }
  if (process.platform === "linux") {
    return "x86_64-unknown-linux-gnu";
  }
  throw new Error(`Unsupported platform: ${process.platform}-${process.arch}`);
}

/** Downloads a release archive through the configured proxy, if one is present. */
async function download(url, destination) {
  const proxy = [
    process.env.HTTPS_PROXY,
    process.env.https_proxy,
    process.env.HTTP_PROXY,
    process.env.http_proxy,
    process.env.ALL_PROXY,
    process.env.all_proxy,
    process.env.PROXY,
    process.env.proxy,
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
async function findExtractedBinary(directory, fileName) {
  const entries = await readdir(directory, { withFileTypes: true });
  for (const entry of entries) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      const nestedPath = await findExtractedBinary(entryPath, fileName);
      if (nestedPath) return nestedPath;
    } else if (entry.name === fileName) {
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
}) {
  const isWindowsTarget = triple.endsWith("-windows-msvc");
  const archivePath = path.join(
    repositoryRoot,
    `${name}-${triple}.${archiveExtension}`,
  );
  const extractDirectory = path.join(binaryDirectory, `.extract-${name}`);
  const destination = path.join(binaryDirectory, executableName);
  if (
    !process.env.CI &&
    !process.argv.includes("--force") &&
    existsSync(destination)
  ) {
    console.log(`${name} sidecar already exists for ${triple}.`);
    return;
  }

  const project = name === "deno" ? "denoland/deno" : "BurntSushi/ripgrep";
  const releaseVersion = version.replace(/^v/, "");
  // Deno tags include a leading "v", while ripgrep release tags do not.
  const releaseTag = name === "deno" ? `v${releaseVersion}` : releaseVersion;
  const url = `https://github.com/${project}/releases/download/${releaseTag}/${asset}`;
  console.log(`Downloading ${name} ${version} for ${triple}...`);
  await download(url, archivePath);
  await rm(extractDirectory, { force: true, recursive: true });
  await mkdir(extractDirectory, { recursive: true });
  await rm(destination, { force: true });
  try {
    if (process.platform === "win32" && archiveExtension === "zip") {
      await execFileAsync("powershell", [
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        `Expand-Archive -LiteralPath '${archivePath}' -DestinationPath '${extractDirectory}' -Force`,
      ]);
    } else if (archiveExtension === "zip") {
      await execFileAsync("unzip", ["-q", archivePath, "-d", extractDirectory]);
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
    await rename(extractedExecutable, destination);
    if (!isWindowsTarget) {
      await chmod(destination, 0o755);
    }
  } finally {
    await rm(archivePath, { force: true });
    await rm(extractDirectory, { force: true, recursive: true });
  }
}

const triple = targetTriple();
const isWindows = triple.endsWith("-windows-msvc");
const denoAsset = `deno-${triple}.zip`;
// The Linux ripgrep release ships a static musl binary, while Tauri still uses
// the host GNU triple in the sidecar filename and environment variables.
const ripgrepAssetTriple =
  triple === "x86_64-unknown-linux-gnu" ? "x86_64-unknown-linux-musl" : triple;
const ripgrepAsset = `ripgrep-${ripgrepVersion}-${ripgrepAssetTriple}.${isWindows ? "zip" : "tar.gz"}`;

await mkdir(binaryDirectory, { recursive: true });
if (binariesToInstall.has("deno")) {
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
