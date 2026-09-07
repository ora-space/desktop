import assert from "node:assert/strict";
import path from "node:path";
import process from "node:process";
import { packageDesktop } from "./package-desktop.ts";
import { withTempDirectory } from "./test-support.ts";

/** Installs distinct snapshots so restoration tests detect accidental rewrites. */
async function packagingFixture(root: string): Promise<Map<string, string>> {
  const files = new Map([
    [
      path.join(root, "apps", "desktop", "src-tauri", "tauri.conf.json"),
      '{"version":"0.1.0","bundle":{"externalBin":[],"createUpdaterArtifacts":false},"app":{"keep":"value"}}\n',
    ],
    [
      path.join(root, "apps", "desktop", "src-tauri", "Cargo.toml"),
      '[package]\nname = "ora"\nversion = "0.1.0"\n',
    ],
    [
      path.join(root, "apps", "desktop", "package.json"),
      '{"version":"0.1.0","private":true}\n',
    ],
    [path.join(root, "Cargo.lock"), "original lock\n"],
  ]);
  for (const [file, text] of files) {
    await Deno.mkdir(path.dirname(file), { recursive: true });
    await Deno.writeTextFile(file, text);
  }
  return files;
}

for (const fails of [false, true]) {
  Deno.test(
    `packaging restores snapshots after ${fails ? "failure" : "success"}`,
    () =>
      withTempDirectory(async (root) => {
        const files = await packagingFixture(root);
        const environment = { TAURI_SIGNING_PRIVATE_KEY: "fixture-key" };
        const listeners = [
          process.listenerCount("SIGINT"),
          process.listenerCount("SIGTERM"),
        ];
        const operation = packageDesktop({
          root,
          args: ["--tag", "refs/tags/v1.2.3", "--signing-key", "override-key"],
          environment,
          runBuild: async (env) => {
            const config = JSON.parse(
              await Deno.readTextFile([...files.keys()][0]),
            );
            assert.deepEqual(config, {
              version: "1.2.3",
              bundle: {
                externalBin: [
                  "binaries/rg",
                  "binaries/deno",
                  "binaries/ora-reaper",
                ],
                createUpdaterArtifacts: true,
              },
              app: { keep: "value" },
            });
            assert.equal(env.TAURI_SIGNING_PRIVATE_KEY, "override-key");
            assert.match(
              await Deno.readTextFile([...files.keys()][1]),
              /version = "1.2.3"/,
            );
            assert.deepEqual(
              JSON.parse(await Deno.readTextFile([...files.keys()][2])),
              { version: "1.2.3", private: true },
            );
            await Deno.writeTextFile(
              path.join(root, "Cargo.lock"),
              "changed by build",
            );
            if (fails) throw new Error("build failed");
          },
        });
        if (fails) await assert.rejects(operation, /build failed/);
        else await operation;
        const actual = new Map<string, string>();
        for (const file of files.keys())
          actual.set(file, await Deno.readTextFile(file));
        assert.deepEqual(actual, files);
        assert.deepEqual(environment, {
          TAURI_SIGNING_PRIVATE_KEY: "fixture-key",
        });
        assert.deepEqual(
          [process.listenerCount("SIGINT"), process.listenerCount("SIGTERM")],
          listeners,
        );
      }),
  );
}

Deno.test("packaging preserves edits only when keep-config is explicit", () =>
  withTempDirectory(async (root) => {
    const files = await packagingFixture(root);
    await packageDesktop({
      root,
      args: ["--keep-config"],
      environment: {},
      runBuild: () => Promise.resolve(),
    });
    assert.deepEqual(
      JSON.parse(await Deno.readTextFile([...files.keys()][0])).bundle,
      {
        externalBin: ["binaries/rg", "binaries/deno", "binaries/ora-reaper"],
        createUpdaterArtifacts: false,
      },
    );
  }),
);

Deno.test("invalid release tags restore files without invoking the build", () =>
  withTempDirectory(async (root) => {
    const files = await packagingFixture(root);
    await assert.rejects(
      packageDesktop({
        root,
        args: ["--tag", "not-a-version"],
        environment: {},
        runBuild: () => {
          throw new Error("must not build");
        },
      }),
      /Invalid --tag/,
    );
    assert.deepEqual(
      await Promise.all(
        [...files.keys()].map((file) => Deno.readTextFile(file)),
      ),
      [...files.values()],
    );
  }),
);

Deno.test("missing option values fail before writing any files", () =>
  withTempDirectory(async (root) => {
    for (const option of ["--tag", "--signing-key", "--signing-key-password"]) {
      await assert.rejects(
        packageDesktop({ root, args: [option], environment: {} }),
        /requires a value/,
      );
    }
    assert.deepEqual([...Deno.readDirSync(root)], []);
  }),
);
