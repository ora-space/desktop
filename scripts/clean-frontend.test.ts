import assert from "node:assert/strict";
import path from "node:path";

/** Runs the real cleanup entrypoint in a disposable checkout, never in this workspace. */
async function runCleanupFixture(member: string) {
  const fixture = await Deno.makeTempDir({ prefix: "ora-clean-test-" });
  const root = path.join(fixture, "checkout");
  await Deno.mkdir(path.join(root, "scripts"), { recursive: true });
  await Deno.copyFile(
    new URL("./clean-frontend.ts", import.meta.url),
    path.join(root, "scripts", "clean-frontend.ts"),
  );
  await Deno.writeTextFile(
    path.join(root, "deno.json"),
    JSON.stringify({ workspace: [member] }),
  );
  for (const directory of [
    path.join(root, "node_modules"),
    path.join(root, "packages", "ui", "node_modules"),
    path.join(root, ".data"),
    path.join(fixture, "outside", "node_modules"),
  ]) {
    await Deno.mkdir(directory, { recursive: true });
    await Deno.writeTextFile(path.join(directory, "marker"), "keep");
  }
  const output = await new Deno.Command(Deno.execPath(), {
    args: [
      "run",
      "--no-config",
      "--no-lock",
      "--allow-read",
      "--allow-write",
      path.join(root, "scripts", "clean-frontend.ts"),
    ],
    stdout: "piped",
    stderr: "piped",
  }).output();
  return { fixture, root, output };
}

Deno.test(
  "cleanup removes workspace dependencies and preserves user and external data",
  async () => {
    const { fixture, root, output } = await runCleanupFixture("packages/ui");
    try {
      assert.equal(output.code, 0, new TextDecoder().decode(output.stderr));
      await assert.rejects(
        Deno.stat(path.join(root, "node_modules")),
        Deno.errors.NotFound,
      );
      await assert.rejects(
        Deno.stat(path.join(root, "packages", "ui", "node_modules")),
        Deno.errors.NotFound,
      );
      assert.deepEqual(
        await Promise.all([
          Deno.readTextFile(path.join(root, ".data", "marker")),
          Deno.readTextFile(
            path.join(fixture, "outside", "node_modules", "marker"),
          ),
        ]),
        ["keep", "keep"],
      );
    } finally {
      await Deno.remove(fixture, { recursive: true });
    }
  },
);

Deno.test(
  "cleanup rejects a workspace member outside the checkout",
  async () => {
    const { fixture, output } = await runCleanupFixture("../outside");
    try {
      assert.notEqual(output.code, 0);
      assert.match(
        new TextDecoder().decode(output.stderr),
        /outside the repository/,
      );
      assert.equal(
        await Deno.readTextFile(
          path.join(fixture, "outside", "node_modules", "marker"),
        ),
        "keep",
      );
    } finally {
      await Deno.remove(fixture, { recursive: true });
    }
  },
);
