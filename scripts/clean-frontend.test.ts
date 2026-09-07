import assert from "node:assert/strict";
import path from "node:path";

/** Builds a disposable checkout for the real cleanup entrypoint, never this workspace. */
async function createCleanupFixture(member: string) {
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
  return { fixture, root };
}

/**
 * Runs the entrypoint as reached through `entrypointRoot`.
 *
 * Taken separately from the checkout it cleans, because the script derives the repository from
 * its own module path: invoking it through a link is how a checkout reached by a rewritten path
 * is exercised.
 */
async function runCleanup(entrypointRoot: string) {
  return await new Deno.Command(Deno.execPath(), {
    args: [
      "run",
      "--no-config",
      "--no-lock",
      "--allow-read",
      "--allow-write",
      path.join(entrypointRoot, "scripts", "clean-frontend.ts"),
    ],
    stdout: "piped",
    stderr: "piped",
  }).output();
}

/**
 * Points `linkPath` at the directory `target`.
 *
 * Windows grants unprivileged symlink creation only under Developer Mode, so it falls back to a
 * junction, which never needs elevation. `realpath` follows both identically, and that is the
 * only property these tests rely on.
 */
async function linkDirectory(target: string, linkPath: string) {
  try {
    await Deno.symlink(target, linkPath, { type: "dir" });
    return;
  } catch (error) {
    if (Deno.build.os !== "windows") throw error;
  }
  const junction = await new Deno.Command("cmd", {
    args: ["/c", "mklink", "/J", linkPath, target],
    stdout: "null",
    stderr: "piped",
  }).output();
  if (junction.code !== 0) {
    throw new Error(
      `mklink /J failed: ${new TextDecoder().decode(junction.stderr)}`,
    );
  }
}

Deno.test(
  "cleanup removes workspace dependencies and preserves user and external data",
  async () => {
    const { fixture, root } = await createCleanupFixture("packages/ui");
    const output = await runCleanup(root);
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
    const { fixture, root } = await createCleanupFixture("../outside");
    const output = await runCleanup(root);
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

Deno.test(
  "cleanup runs in a checkout reached through a rewritten path",
  async () => {
    const { fixture, root } = await createCleanupFixture("packages/ui");
    // Stands in for every path `realpath` rewrites: a linked parent here, and on Windows an 8.3
    // short name for a profile whose real name contains a space. The checkout is legitimate and
    // must clean, rather than reading as a member that escaped it.
    const link = path.join(fixture, "link");
    await linkDirectory(root, link);
    const output = await runCleanup(link);
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
    } finally {
      await Deno.remove(fixture, { recursive: true });
    }
  },
);

Deno.test(
  "cleanup rejects a workspace member linked outside the checkout",
  async () => {
    // Names a member that is lexically inside the checkout, so only following the link can
    // reveal the escape. This is the case the resolved comparison exists for.
    const { fixture, root } = await createCleanupFixture("linked");
    await linkDirectory(
      path.join(fixture, "outside"),
      path.join(root, "linked"),
    );
    const output = await runCleanup(root);
    try {
      assert.notEqual(output.code, 0);
      assert.match(
        new TextDecoder().decode(output.stderr),
        /resolves outside the repository/,
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
