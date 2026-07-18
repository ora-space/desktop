type BuildArtifact = Blob & {
  readonly kind: string;
};

type BuildOutput = {
  readonly success: boolean;
  readonly logs: readonly unknown[];
  readonly outputs: readonly BuildArtifact[];
  readonly metafile?: unknown;
};

declare const Bun: {
  readonly argv: readonly string[];
  build(options: {
    readonly entrypoints: readonly string[];
    readonly target: "bun";
    readonly format: "esm";
    readonly packages: "bundle";
    readonly metafile: true;
    readonly allowUnresolved: readonly string[];
  }): Promise<BuildOutput>;
  write(path: string, data: Blob | string): Promise<number>;
};

/** Runs the fixed Bun API build with opaque dynamic specifiers forbidden. */
async function main(): Promise<void> {
  const [, , entry, output, metafile] = Bun.argv;
  if (!entry || !output || !metafile) {
    throw new Error("artifact builder requires entry, output, and metafile paths");
  }
  const result = await Bun.build({
    entrypoints: [entry],
    target: "bun",
    format: "esm",
    packages: "bundle",
    metafile: true,
    allowUnresolved: [],
  });
  const entries = result.outputs.filter((artifact) => artifact.kind === "entry-point");
  if (!result.success || result.outputs.length !== 1 || entries.length !== 1 || !result.metafile) {
    throw new Error(`materialized build failed: ${JSON.stringify(result.logs).slice(0, 4096)}`);
  }
  await Bun.write(output, entries[0]);
  await Bun.write(metafile, JSON.stringify(result.metafile));
}

await main();
