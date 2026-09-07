import path from "node:path";

/** Isolates filesystem effects and always removes the disposable test directory. */
export async function withTempDirectory(
  run: (root: string) => Promise<void>,
): Promise<void> {
  const root = await Deno.makeTempDir({ prefix: "ora-script-test-" });
  try {
    await run(root);
  } finally {
    await Deno.remove(root, { recursive: true });
  }
}

/** Runs a CLI without changing the test process's working directory or environment. */
export async function runScript(
  name: string,
  cwd: string,
  args: string[] = [],
): Promise<{ code: number; stdout: string; stderr: string }> {
  const output = await new Deno.Command(Deno.execPath(), {
    args: ["run", "-A", new URL(name, import.meta.url).href, ...args],
    cwd,
    stdout: "piped",
    stderr: "piped",
  }).output();
  return {
    code: output.code,
    stdout: new TextDecoder().decode(output.stdout),
    stderr: new TextDecoder().decode(output.stderr),
  };
}

/** Creates an independent index and a real formatter task for hook tests. */
export async function initFormatterRepository(root: string): Promise<void> {
  await git(root, ["init", "--quiet"]);
  await Deno.writeTextFile(
    path.join(root, "package.json"),
    JSON.stringify({
      scripts: {
        "format:files": `deno run -A ${new URL("../node_modules/prettier/bin/prettier.cjs", import.meta.url).href}`,
      },
    }),
  );
}

/** Fails fixture setup immediately if Git cannot create the intended index state. */
export async function git(root: string, args: string[]): Promise<void> {
  const result = await new Deno.Command("git", {
    args: ["-C", root, ...args],
    stdout: "piped",
    stderr: "piped",
  }).output();
  if (!result.success) throw new Error(new TextDecoder().decode(result.stderr));
}
