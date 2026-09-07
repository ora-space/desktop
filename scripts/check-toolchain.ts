import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url));

/** Requires an exact release so CI and downloaded sidecars cannot resolve different versions. */
export async function readDenoVersion(root = repositoryRoot): Promise<string> {
  const version = (
    await readFile(path.join(root, ".deno-version"), "utf8")
  ).trim();
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(".deno-version must contain an exact stable Deno version.");
  }
  return version;
}

/** Rejects runtime drift and package-local compiler pins before checks or packaging begin. */
export async function checkToolchain(
  root = repositoryRoot,
  runningDeno = Deno.version.deno,
): Promise<void> {
  const expected = await readDenoVersion(root);
  if (runningDeno !== expected) {
    throw new Error(
      `Expected Deno ${expected}, found ${runningDeno}. Run deno upgrade --version ${expected}.`,
    );
  }
  const manifest = JSON.parse(
    await readFile(path.join(root, "package.json"), "utf8"),
  );
  if (manifest.engines?.deno !== expected) {
    throw new Error("package.json engines.deno must match .deno-version.");
  }
  const workspace = JSON.parse(
    await readFile(path.join(root, "deno.json"), "utf8"),
  );
  for (const member of workspace.workspace) {
    const memberManifest = JSON.parse(
      await readFile(path.join(root, member, "package.json"), "utf8"),
    );
    for (const dependencies of [
      memberManifest.dependencies,
      memberManifest.devDependencies,
    ]) {
      if (dependencies?.typescript) {
        throw new Error(
          `${member} must use the compiler configured in the root package.json.`,
        );
      }
    }
  }
}

if (import.meta.main) await checkToolchain();
