import assert from "node:assert/strict";
import path from "node:path";
import { buildReaper } from "./build-reaper.ts";
import { withTempDirectory } from "./test-support.ts";

for (const scenario of [
  {
    name: "host debug",
    args: [],
    environment: {},
    target: "x86_64-unknown-linux-gnu",
    relative: ["target", "debug", "ora-reaper"],
    command: ["build", "--package", "ora-reaper"],
  },
  {
    name: "cross release with custom target directory",
    args: ["--release"],
    environment: {
      TARGET_TRIPLE: "x86_64-pc-windows-msvc",
      CARGO_TARGET_DIR: "custom target",
    },
    target: "x86_64-pc-windows-msvc",
    relative: [
      "custom target",
      "x86_64-pc-windows-msvc",
      "release",
      "ora-reaper.exe",
    ],
    command: [
      "build",
      "--package",
      "ora-reaper",
      "--release",
      "--target",
      "x86_64-pc-windows-msvc",
    ],
  },
]) {
  Deno.test(`reaper installs ${scenario.name}`, () =>
    withTempDirectory(async (root) => {
      const calls: unknown[] = [];
      await buildReaper({
        root,
        args: scenario.args,
        environment: scenario.environment,
        hostTarget: () => Promise.resolve("x86_64-unknown-linux-gnu"),
        run: async (command, args) => {
          calls.push({ command, args });
          const source = path.join(root, ...scenario.relative);
          await Deno.mkdir(path.dirname(source), { recursive: true });
          await Deno.writeTextFile(source, "reaper");
        },
      });
      assert.deepEqual(calls, [{ command: "cargo", args: scenario.command }]);
      assert.equal(
        await Deno.readTextFile(
          path.join(
            root,
            "apps",
            "desktop",
            "src-tauri",
            "binaries",
            `ora-reaper-${scenario.target}${scenario.target.includes("windows") ? ".exe" : ""}`,
          ),
        ),
        "reaper",
      );
    }),
  );
}

Deno.test("reaper propagates a failed build without installing a sidecar", () =>
  withTempDirectory(async (root) => {
    await assert.rejects(
      buildReaper({
        root,
        args: [],
        environment: { RUST_TARGET: "target" },
        run: () => Promise.reject(new Error("cargo failed")),
      }),
      /cargo failed/,
    );
    await assert.rejects(
      Deno.stat(path.join(root, "apps")),
      Deno.errors.NotFound,
    );
  }),
);
