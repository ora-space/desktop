import assert from "node:assert/strict";
import path from "node:path";
import {
  git,
  initFormatterRepository,
  runScript,
  withTempDirectory,
} from "./test-support.ts";

Deno.test("format hook checks the index rather than unstaged fixes", () =>
  withTempDirectory(async (root) => {
    await initFormatterRepository(root);
    const file = path.join(root, "staged file.ts");
    await Deno.writeTextFile(file, "const x=1");
    await git(root, ["add", "staged file.ts"]);
    await Deno.writeTextFile(file, "const x = 1;\n");
    const failed = await runScript("check-staged-format.ts", root);
    assert.equal(failed.code, 1);
    assert.match(failed.stderr, /staged file.ts/);
    await git(root, ["add", "staged file.ts"]);
    assert.deepEqual(await runScript("check-staged-format.ts", root), {
      code: 0,
      stdout: "",
      stderr: "",
    });
  }),
);

Deno.test("format hook leaves generated Rust to its generator", () =>
  withTempDirectory(async (root) => {
    await initFormatterRepository(root);
    await Deno.writeTextFile(
      path.join(root, "generated.rs"),
      '// @generated\n#[prost(tag="1")]\npub struct A{}\n',
    );
    await git(root, ["add", "generated.rs"]);
    assert.deepEqual(await runScript("check-staged-format.ts", root), {
      code: 0,
      stdout: "",
      stderr: "",
    });
    await Deno.writeTextFile(
      path.join(root, "handwritten.rs"),
      '#[prost(tag="1")]\npub struct A{}\n',
    );
    await git(root, ["add", "handwritten.rs"]);
    const failed = await runScript("check-staged-format.ts", root);
    assert.equal(failed.code, 1);
    assert.match(failed.stderr, /handwritten.rs/);
    assert.doesNotMatch(failed.stderr, /generated.rs/);
  }),
);

Deno.test("format hook accepts an empty index", () =>
  withTempDirectory(async (root) => {
    await initFormatterRepository(root);
    assert.deepEqual(await runScript("check-staged-format.ts", root), {
      code: 0,
      stdout: "",
      stderr: "",
    });
  }),
);
