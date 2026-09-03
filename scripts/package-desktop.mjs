// Local equivalent of the `bundle` job in .github/workflows/desktop-build.yml.
//
// The CI workflow edits apps/desktop/src-tauri/tauri.conf.json in place on a
// disposable runner and never restores it. A developer's checkout is not
// disposable, so this script snapshots every file it touches and restores it
// once `task build:desktop` finishes (success, failure, or interruption) unless
// --keep-config is passed.
//
// Usage:
//   node scripts/package-desktop.mjs [--tag v0.1.0] [--signing-key <path-or-key>]
//     [--signing-key-password <password>] [--keep-config]
//
// Signed updater artifacts are produced when a signing key is supplied, via
// --signing-key or the TAURI_SIGNING_PRIVATE_KEY environment variable it
// mirrors, exactly like the CI job.
import { readFile, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import path from "node:path";
import process from "node:process";

const repositoryRoot = path.resolve(import.meta.dirname, "..");
const tauriConfigPath = path.join(
  repositoryRoot,
  "apps",
  "desktop",
  "src-tauri",
  "tauri.conf.json",
);
const desktopCargoPath = path.join(
  repositoryRoot,
  "apps",
  "desktop",
  "src-tauri",
  "Cargo.toml",
);
const desktopPackagePath = path.join(
  repositoryRoot,
  "apps",
  "desktop",
  "package.json",
);
const cargoLockPath = path.join(repositoryRoot, "Cargo.lock");

const args = process.argv.slice(2);
const keepConfig = args.includes("--keep-config");
const tagIndex = args.indexOf("--tag");
const tag = tagIndex !== -1 ? args[tagIndex + 1] : undefined;
const signingKeyIndex = args.indexOf("--signing-key");
const signingKey =
  signingKeyIndex !== -1 ? args[signingKeyIndex + 1] : undefined;
const signingKeyPasswordIndex = args.indexOf("--signing-key-password");
const signingKeyPassword =
  signingKeyPasswordIndex !== -1
    ? args[signingKeyPasswordIndex + 1]
    : undefined;
if (args.includes("--help") || args.includes("-h")) {
  console.log(
    "Usage: node scripts/package-desktop.mjs [options]\n\n" +
      "Builds the Tauri desktop bundle(s) for the current platform, the same\n" +
      "way .github/workflows/desktop-build.yml does. Bundles land in\n" +
      "target/release/bundle/.\n\n" +
      "Options:\n" +
      "  --tag <v0.1.0>                 Set the app version for this build\n" +
      "                                 (tauri.conf.json, Cargo.toml, package.json).\n" +
      "                                 Defaults to the checked-in version.\n" +
      "  --signing-key <path-or-key>    Private key for signed updater artifacts.\n" +
      "                                 Accepts a path to a .key file or the key's\n" +
      "                                 raw contents. Must match the pubkey baked\n" +
      "                                 into tauri.conf.json. Same as setting\n" +
      "                                 TAURI_SIGNING_PRIVATE_KEY; the flag wins if\n" +
      "                                 both are set. Omit for an unsigned build.\n" +
      "  --signing-key-password <pass>  Password for an encrypted --signing-key.\n" +
      "                                 Same as TAURI_SIGNING_PRIVATE_KEY_PASSWORD;\n" +
      "                                 the flag wins if both are set.\n" +
      "  --keep-config                  Leave the working-copy edits (tauri.conf.json,\n" +
      "                                 Cargo.toml, package.json, Cargo.lock) in place\n" +
      "                                 afterward instead of restoring them.\n" +
      "  --help, -h                     Show this message.",
  );
  process.exit(0);
}
if (tagIndex !== -1 && !tag) {
  throw new Error("--tag requires a value, e.g. --tag v0.1.0");
}
if (signingKeyIndex !== -1 && !signingKey) {
  throw new Error(
    "--signing-key requires a value (a path or the key contents)",
  );
}
if (signingKeyPasswordIndex !== -1 && !signingKeyPassword) {
  throw new Error("--signing-key-password requires a value");
}
if (signingKey) process.env.TAURI_SIGNING_PRIVATE_KEY = signingKey;
if (signingKeyPassword)
  process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = signingKeyPassword;

let activeChild;
let interruptedSignal;

/** Runs one build step with its output attached to this process. */
function run(command, commandArgs, env) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, commandArgs, {
      cwd: repositoryRoot,
      stdio: "inherit",
      env: { ...process.env, ...env },
    });
    activeChild = child;
    child.once("error", (error) => {
      activeChild = undefined;
      reject(error);
    });
    child.once("exit", (code, signal) => {
      activeChild = undefined;
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

/** Stops the active build so the outer cleanup can restore its snapshots. */
function handleTerminationSignal(signal) {
  if (interruptedSignal) return;

  interruptedSignal = signal;
  console.error(`\nReceived ${signal}; stopping the build before cleanup.`);
  activeChild?.kill(signal);
}

const terminationHandlers = new Map(
  ["SIGINT", "SIGTERM"].map((signal) => [
    signal,
    () => handleTerminationSignal(signal),
  ]),
);
for (const [signal, handler] of terminationHandlers) {
  process.on(signal, handler);
}

/** Prevents an interrupted setup phase from starting the expensive build. */
function throwIfInterrupted() {
  if (interruptedSignal) {
    throw new Error(`Packaging interrupted by ${interruptedSignal}`);
  }
}

const originalFiles = new Map();

/** Reads a file's current contents so it can be restored later. */
async function snapshot(filePath) {
  originalFiles.set(filePath, await readFile(filePath, "utf8"));
}

/** Restores every snapshotted file to its pre-build contents. */
async function restoreSnapshots() {
  if (keepConfig) {
    console.log("--keep-config set; leaving working-copy edits in place.");
    return;
  }
  for (const [filePath, contents] of originalFiles) {
    await writeFile(filePath, contents);
  }
  console.log(
    "Restored tauri.conf.json, Cargo.toml, package.json, and Cargo.lock.",
  );
}

// Sets the desktop app version across tauri.conf.json, Cargo.toml, and
// package.json, run in-process so we can snapshot the files it edits before
// touching them.
async function applyVersionTag(versionTag) {
  const version = versionTag.replace(/^refs\/tags\//, "").replace(/^v/, "");
  if (
    !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version)
  ) {
    throw new Error(`Invalid --tag version: ${versionTag}`);
  }

  const tauriConfig = JSON.parse(originalFiles.get(tauriConfigPath));
  tauriConfig.version = version;
  await writeFile(tauriConfigPath, `${JSON.stringify(tauriConfig, null, 2)}\n`);

  const cargoManifest = originalFiles.get(desktopCargoPath);
  const updatedCargoManifest = cargoManifest.replace(
    /(^\[package\][\s\S]*?^version\s*=\s*")([^"]+)(")/m,
    `$1${version}$3`,
  );
  if (updatedCargoManifest === cargoManifest) {
    throw new Error(
      `Could not find the package version in ${desktopCargoPath}`,
    );
  }
  await writeFile(desktopCargoPath, updatedCargoManifest);

  const desktopPackage = JSON.parse(originalFiles.get(desktopPackagePath));
  desktopPackage.version = version;
  await writeFile(
    desktopPackagePath,
    `${JSON.stringify(desktopPackage, null, 2)}\n`,
  );

  console.log(`Set desktop version to ${version} from ${versionTag}.`);
}

// Mirrors the "Configure Tauri sidecars" and "Configure Tauri updater
// artifacts" steps in desktop-build.yml.
async function configureBundle() {
  const config = JSON.parse(await readFile(tauriConfigPath, "utf8"));
  config.bundle.externalBin = [
    "binaries/rg",
    "binaries/deno",
    "binaries/ora-reaper",
  ];
  const hasSigningKey = Boolean(process.env.TAURI_SIGNING_PRIVATE_KEY?.trim());
  config.bundle.createUpdaterArtifacts = hasSigningKey;
  if (!hasSigningKey) {
    console.log(
      "TAURI_SIGNING_PRIVATE_KEY is not set; building without updater artifacts.",
    );
  }
  await writeFile(tauriConfigPath, `${JSON.stringify(config, null, 2)}\n`);
}

let buildError;
try {
  await snapshot(tauriConfigPath);
  await snapshot(desktopCargoPath);
  await snapshot(desktopPackagePath);
  // Changing a workspace package version makes Cargo update this generated
  // entry during the build, so it belongs to the same temporary transaction.
  await snapshot(cargoLockPath);

  throwIfInterrupted();
  if (tag) await applyVersionTag(tag);
  await configureBundle();

  throwIfInterrupted();
  await run("task", ["build:desktop"]);

  console.log("\nBundle(s) written to target/release/bundle/");
} catch (error) {
  buildError = error;
} finally {
  try {
    await restoreSnapshots();
  } finally {
    for (const [signal, handler] of terminationHandlers) {
      process.off(signal, handler);
    }
  }
}

if (interruptedSignal) {
  process.kill(process.pid, interruptedSignal);
}
if (buildError) {
  throw buildError;
}
