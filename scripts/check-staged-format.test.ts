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
