import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** Checks the second generated layer in isolation so verification never repairs a stale checkout. */
export async function checkContractSchema(): Promise<void> {
  const root = fileURLToPath(new URL("../", import.meta.url));
  const sourceDirectory = path.join(root, "packages", "contracts", "src");
  const manifest = JSON.parse(
    await readFile(path.join(sourceDirectory, "..", "package.json"), "utf8"),
  );
  const generatorVersion: string = manifest.devDependencies["ts-to-zod"];
  const temporary = await mkdtemp(path.join(tmpdir(), "ora-contract-schema-"));
  try {
    const output = path.join(temporary, "error.schema.ts");
    const generated = spawnSync(
      Deno.execPath(),
      [
        "run",
        "-A",
        `npm:ts-to-zod@${generatorVersion}`,
        path.relative(root, path.join(sourceDirectory, "error.ts")),
        path.relative(root, output),
      ],
      { cwd: root, encoding: "utf8" },
    );
    if (generated.status !== 0)
      throw new Error(
        `Schema generation failed:\n${generated.stdout}\n${generated.stderr}`,
      );
    const [expected, current] = await Promise.all([
      readFile(output, "utf8"),
      readFile(path.join(sourceDirectory, "error.schema.ts"), "utf8"),
    ]);
    if (current !== expected)
      throw new Error(
        "Generated error.schema.ts differs; run task export-contracts.",
      );
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

if (import.meta.main) await checkContractSchema();
