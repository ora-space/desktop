import assert from "node:assert/strict";
import path from "node:path";
import {
  git,
  initFormatterRepository,
  runScript,
  withTempDirectory,
} from "./test-support.ts";

Deno.test(
  "formatter handles staged and untracked filenames containing spaces",
  () =>
    withTempDirectory(async (root) => {
      await initFormatterRepository(root);
      await Deno.writeTextFile(path.join(root, "staged file.ts"), "const x=1");
      await git(root, ["add", "staged file.ts"]);
      await Deno.writeTextFile(
        path.join(root, "untracked file.ts"),
        "const y=2",
      );
      const output = await runScript("format-changed.ts", root);
      assert.equal(output.code, 0, output.stderr);
      assert.deepEqual(
        await Promise.all(
          ["staged file.ts", "untracked file.ts"].map((file) =>
            Deno.readTextFile(path.join(root, file)),
          ),
        ),
        ["const x = 1;\n", "const y = 2;\n"],
      );
    }),
);

Deno.test("formatter reports invalid source as a failure", () =>
  withTempDirectory(async (root) => {
    await initFormatterRepository(root);
    await Deno.writeTextFile(path.join(root, "invalid.ts"), "const =");
    assert.notEqual((await runScript("format-changed.ts", root)).code, 0);
  }),
);
