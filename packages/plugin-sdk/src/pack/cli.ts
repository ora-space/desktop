#!/usr/bin/env node

import { packAgentPlugin, validateMaterializedArtifact } from "./index.js";

/** Implements the intentionally small pack/validate command-line surface. */
async function main(args: readonly string[]): Promise<void> {
  const [command, ...rest] = args;
  const options = parseOptions(rest);
  if (command === "pack") {
    const sourceRoot = required(options, "source");
    const entry = required(options, "entry");
    const outputRoot = required(options, "output");
    const result = await packAgentPlugin({
      sourceRoot,
      entry,
      outputRoot,
      bunExecutable: options.get("bun"),
    });
    process.stdout.write(`${JSON.stringify(result)}\n`);
    return;
  }
  if (command === "validate") {
    await validateMaterializedArtifact({
      artifactRoot: required(options, "artifact"),
      metafilePath: options.get("metafile"),
      bunExecutable: options.get("bun"),
    });
    process.stdout.write("valid\n");
    return;
  }
  throw new Error("usage: ora-plugin-pack <pack|validate> --key value");
}

function parseOptions(args: readonly string[]): Map<string, string> {
  const options = new Map<string, string>();
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    const value = args[index + 1];
    if (!key?.startsWith("--") || !value || value.startsWith("--")) {
      throw new Error("pack options must use --key value pairs");
    }
    const normalized = key.slice(2);
    if (options.has(normalized)) {
      throw new Error(`duplicate option: ${key}`);
    }
    options.set(normalized, value);
  }
  return options;
}

function required(options: ReadonlyMap<string, string>, key: string): string {
  const value = options.get(key);
  if (!value) {
    throw new Error(`missing --${key}`);
  }
  return value;
}

await main(process.argv.slice(2));
