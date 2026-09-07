import fs from "node:fs/promises";
import path from "node:path";

export interface UpdateManifest {
  version: string;
  notes: string;
  pub_date: string;
  platforms: Record<string, { url: string; signature: string }>;
}

/** Creates the public updater document only after all signed platform assets are available. */
export async function createUpdateManifest(options: {
  bundlesDirectory: string;
  releaseTag: string | undefined;
  now?: () => Date;
}): Promise<UpdateManifest> {
  const { bundlesDirectory, releaseTag } = options;
  if (!releaseTag)
    throw new Error("RELEASE_TAG is required to create latest.json");
  const assets = new Map<string, string>();
  for (const file of await collectFiles(bundlesDirectory)) {
    const name = path.basename(file);
    if (assets.has(name))
      throw new Error(`Duplicate updater asset name: ${name}`);
    assets.set(name, file);
  }
  const patterns = {
    "windows-x86_64": /(?:_x64-setup|setup)\.exe$/i,
    "darwin-aarch64": /\.app\.tar\.gz$/i,
    "linux-x86_64": /\.AppImage$/,
  };
  const manifest: UpdateManifest = {
    version: releaseTag.replace(/^v/, ""),
    notes: `Ora ${releaseTag}`,
    pub_date: (options.now ?? (() => new Date()))().toISOString(),
    platforms: {},
  };
  for (const [target, pattern] of Object.entries(patterns)) {
    const matches = [...assets].filter(
      ([name]) => pattern.test(name) && assets.has(`${name}.sig`),
    );
    if (matches.length !== 1) {
      throw new Error(
        `Expected one signed updater asset matching ${pattern}, found ${matches.length}`,
      );
    }
    const [name, file] = matches[0];
    manifest.platforms[target] = {
      url: `https://github.com/ora-space/desktop/releases/download/${encodeURIComponent(releaseTag)}/${encodeURIComponent(name)}`,
      signature: await fs.readFile(`${file}.sig`, "utf8"),
    };
  }
  await fs.writeFile(
    path.join(bundlesDirectory, "latest.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  return manifest;
}

/** Traverses artifact directories so both CI downloads and local bundle layouts work. */
async function collectFiles(directory: string): Promise<string[]> {
  const entries = await fs.readdir(directory, { withFileTypes: true });
  const files: string[] = [];
  for (const entry of entries) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await collectFiles(entryPath)));
    else if (entry.isFile()) files.push(entryPath);
  }
  return files;
}

if (import.meta.main) {
  await createUpdateManifest({
    bundlesDirectory: path.resolve(Deno.args[0] ?? "bundles"),
    releaseTag: Deno.env.get("RELEASE_TAG"),
  });
}
