type BunImport = {
  readonly path: string;
  readonly kind: string;
};

declare const Bun: {
  readonly argv: readonly string[];
  file(path: string): { text(): Promise<string> };
  Transpiler: new (options: { loader: "js" }) => {
    scan(code: string): { imports: BunImport[] };
  };
};

/** Parses one bundled entry with Bun's ECMAScript parser without importing or executing it. */
async function main(): Promise<void> {
  const entry = Bun.argv[2];
  if (!entry) {
    throw new Error("artifact scanner requires an entry path");
  }
  const source = await Bun.file(entry).text();
  const imports = new Bun.Transpiler({ loader: "js" }).scan(source).imports;
  process.stdout.write(JSON.stringify(imports));
}

await main();
