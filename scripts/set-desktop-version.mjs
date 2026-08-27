import fs from "node:fs/promises";
import path from "node:path";

const rootDirectory = path.resolve(import.meta.dirname, "..");
const versionSource = process.env.VERSION_TAG ?? process.argv[2];

if (!versionSource) {
  console.log(
    "No release tag supplied; keeping the checked-in desktop version.",
  );
  process.exit(0);
}

const version = versionSource.replace(/^refs\/tags\//, "").replace(/^v/, "");

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error(`Invalid release tag/version: ${versionSource}`);
}

const tauriConfigPath = path.join(
  rootDirectory,
  "apps",
  "desktop",
  "src-tauri",
  "tauri.conf.json",
);
const desktopCargoPath = path.join(
  rootDirectory,
  "apps",
  "desktop",
  "src-tauri",
  "Cargo.toml",
);
const desktopPackagePath = path.join(
  rootDirectory,
  "apps",
  "desktop",
  "package.json",
);

const tauriConfig = JSON.parse(await fs.readFile(tauriConfigPath, "utf8"));
tauriConfig.version = version;
await fs.writeFile(
  tauriConfigPath,
  `${JSON.stringify(tauriConfig, null, 2)}\n`,
);

const cargoManifest = await fs.readFile(desktopCargoPath, "utf8");
const updatedCargoManifest = cargoManifest.replace(
  /(^\[package\][\s\S]*?^version\s*=\s*")([^"]+)(")/m,
  `$1${version}$3`,
);

if (updatedCargoManifest === cargoManifest) {
  throw new Error(`Could not find the package version in ${desktopCargoPath}`);
}

await fs.writeFile(desktopCargoPath, updatedCargoManifest);

const desktopPackage = JSON.parse(
  await fs.readFile(desktopPackagePath, "utf8"),
);
desktopPackage.version = version;
await fs.writeFile(
  desktopPackagePath,
  `${JSON.stringify(desktopPackage, null, 2)}\n`,
);

console.log(`Set desktop version to ${version} from ${versionSource}.`);
