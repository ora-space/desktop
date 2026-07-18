import assert from "node:assert/strict";
import { mkdir, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { packAgentPlugin, validateMaterializedArtifact } from "../../src/pack/index.js";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const workspaceRoot = resolve(packageRoot, "..", "..");
const bunExecutable = join(workspaceRoot, "runtime-assets", "prepared", "bun.exe");
const testRoot = join(workspaceRoot, ".cache", "plugin-sdk-pack-tests");

test("pack materializes a public-import plugin outside its source", async () => {
  const source = join(testRoot, "source");
  const output = join(testRoot, "输出", "artifact");
  await resetTestRoot();
  await writeSourceFixture(source, validEntry());
  await linkPublicSdk(source);

  const result = await packAgentPlugin({
    sourceRoot: source,
    entry: "src/index.ts",
    outputRoot: output,
    bunExecutable,
  });
  const bundled = await readFile(join(output, "dist", "index.js"), "utf8");

  assert.equal(result.artifactRoot, output);
  assert.equal(bundled.includes("defineAgentPlugin"), true);
  await validateMaterializedArtifact({
    artifactRoot: output,
    metafilePath: result.metafilePath,
    bunExecutable,
  });
});

test("pack rejects output inside input and unresolved dynamic imports", async () => {
  const source = join(testRoot, "malicious-source");
  await resetTestRoot();
  await writeSourceFixture(source, "const target = process.argv[0]; export default import(target);");

  await assert.rejects(
    packAgentPlugin({
      sourceRoot: source,
      entry: "src/index.ts",
      outputRoot: join(source, "artifact"),
      bunExecutable,
    }),
    /outside the source tree/,
  );
  await assert.rejects(
    packAgentPlugin({
      sourceRoot: source,
      entry: "src/index.ts",
      outputRoot: join(testRoot, "malicious-output"),
      bunExecutable,
    }),
    /will not be bundled|retains dynamic-import import/,
  );
});

test("validator rejects native addons, links, and non-allowlisted files", async () => {
  const artifact = join(testRoot, "invalid-artifact");
  await resetTestRoot();
  await mkdir(join(artifact, "dist"), { recursive: true });
  await writeFile(join(artifact, "package.json"), fixturePackageJson(), "utf8");
  await writeFile(join(artifact, "dist", "index.js"), "export default {};", "utf8");
  await writeFile(join(artifact, "addon.node"), "native", "utf8");

  await assert.rejects(
    validateMaterializedArtifact({ artifactRoot: artifact, bunExecutable }),
    /not allowed/,
  );
  await rm(join(artifact, "addon.node"));
  await symlink(join(artifact, "dist"), join(artifact, "linked-dir"), "junction");
  await assert.rejects(
    validateMaterializedArtifact({ artifactRoot: artifact, bunExecutable }),
    /link/,
  );
});

async function resetTestRoot(): Promise<void> {
  await rm(testRoot, { recursive: true, force: true });
  await mkdir(testRoot, { recursive: true });
}

async function writeSourceFixture(root: string, entry: string): Promise<void> {
  await mkdir(join(root, "src"), { recursive: true });
  await writeFile(join(root, "package.json"), fixturePackageJson(), "utf8");
  await writeFile(join(root, "src", "index.ts"), entry, "utf8");
}

async function linkPublicSdk(source: string): Promise<void> {
  const scope = join(source, "node_modules", "@ora-space");
  await mkdir(scope, { recursive: true });
  await symlink(packageRoot, join(scope, "plugin-sdk"), "junction");
}

function fixturePackageJson(): string {
  return JSON.stringify({
    name: "@ora/pack-fixture",
    version: "0.1.0",
    type: "module",
    ora: {
      manifestVersion: 1,
      id: "ora.pack-fixture",
      displayName: "Pack Fixture",
      kind: "agent",
      main: "dist/index.js",
      engines: {
        ora: ">=0.1.0 <0.2.0",
        pluginApi: 1,
        bun: ">=1.0.0 <2.0.0",
      },
      contributes: { agents: [{ id: "example", displayName: "Example", contractVersion: 1 }] },
    },
  });
}

function validEntry(): string {
  return `
import { defineAgentPlugin } from "@ora-space/plugin-sdk/agent";
export default defineAgentPlugin({
  kind: "agent",
  pluginApi: 1,
  async activate() { return { providers: [] }; }
});
`;
}
