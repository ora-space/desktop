import assert from "node:assert/strict";
import path from "node:path";
import { checkToolchain, readDenoVersion } from "./check-toolchain.ts";
import { withTempDirectory } from "./test-support.ts";

Deno.test(
  "toolchain check rejects runtime, manifest, and child compiler drift",
  () =>
    withTempDirectory(async (root) => {
      await Deno.writeTextFile(path.join(root, ".deno-version"), "2.9.5\n");
      await Deno.writeTextFile(
        path.join(root, "package.json"),
        JSON.stringify({ engines: { deno: "2.9.5" } }),
      );
      await Deno.writeTextFile(
        path.join(root, "deno.json"),
        JSON.stringify({ workspace: ["child"] }),
      );
      await Deno.mkdir(path.join(root, "child"));
      const childManifest = path.join(root, "child", "package.json");
      await Deno.writeTextFile(childManifest, "{}");
      await checkToolchain(root, "2.9.5");
      await assert.rejects(
        checkToolchain(root, "2.9.4"),
        /Expected Deno 2.9.5, found 2.9.4/,
      );
      await Deno.writeTextFile(
        childManifest,
        JSON.stringify({ devDependencies: { typescript: "5.9.3" } }),
      );
      await assert.rejects(
        checkToolchain(root, "2.9.5"),
        /child must use the compiler/,
      );
      await Deno.writeTextFile(childManifest, "{}");
      await Deno.writeTextFile(
        path.join(root, "package.json"),
        JSON.stringify({ engines: { deno: "2.9.4" } }),
      );
      await assert.rejects(
        checkToolchain(root, "2.9.5"),
        /engines.deno must match/,
      );
      await Deno.writeTextFile(path.join(root, ".deno-version"), "2.x\n");
      await assert.rejects(readDenoVersion(root), /exact stable Deno version/);
    }),
);
