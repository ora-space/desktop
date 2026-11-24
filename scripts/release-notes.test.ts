import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  MAX_RELEASE_NOTE_TEXT_LENGTH,
  parseReleaseNote,
  readReleaseNotes,
  requireReleaseNotes,
} from "./release-notes.ts";
import {
  commitAll,
  git,
  releaseNote,
  withTempDirectory,
} from "./test-support.ts";

/** Writes notes files below a disposable checkout. */
async function writeNotes(
  root: string,
  files: Record<string, unknown>,
): Promise<void> {
  const directory = path.join(root, "release-notes");
  await Deno.mkdir(directory, { recursive: true });
  for (const [name, content] of Object.entries(files)) {
    await Deno.writeTextFile(
      path.join(directory, name),
      typeof content === "string" ? content : JSON.stringify(content),
    );
  }
}

// Runs in the existing tooling tests, so a pull request that only edits notes is validated too.
Deno.test("the repository's release notes are valid", async () => {
  await readReleaseNotes(fileURLToPath(new URL("..", import.meta.url)));
});

Deno.test("a complete notes file keeps every field", () => {
  const note = {
    ...releaseNote("0.7.0"),
    entries: ["important", "feature", "improvement", "fix"].map((category) => ({
      category,
      "zh-CN": "字".repeat(MAX_RELEASE_NOTE_TEXT_LENGTH),
      "en-US": "<b>kept</b> as [text](https://example.invalid)",
    })),
  };
  assert.deepEqual(parseReleaseNote("0.7.0.json", JSON.stringify(note)), note);
});

const valid = releaseNote("0.7.0");
const [entry] = valid.entries;
const invalidNotes: [string, unknown, RegExp][] = [
  ["broken JSON", "{", /invalid JSON/],
  ["a non-object document", [], /must be an object/],
  ["an unsupported schema", { ...valid, schema: 2 }, /schema must be 1/],
  ["a missing field", { ...valid, date: undefined }, /missing date/],
  ["source pull requests", { ...valid, prs: [1] }, /unknown fields: prs/],
  [
    "a mismatched version",
    { ...valid, version: "0.7.1" },
    /file name must be 0\.7\.1\.json/,
  ],
  ["a v prefix", { ...valid, version: "v0.7.0" }, /stable version/],
  [
    "a prerelease version",
    { ...valid, version: "0.7.0-rc.1" },
    /stable version/,
  ],
  ["a leading zero", { ...valid, version: "0.07.0" }, /stable version/],
  ["an impossible date", { ...valid, date: "2026-02-30" }, /calendar date/],
  ["a date with time", { ...valid, date: "2026-12-10T00:00" }, /calendar date/],
  ["no entries", { ...valid, entries: [] }, /non-empty array/],
  [
    "an unknown category",
    { ...valid, entries: [{ ...entry, category: "other" }] },
    /category must be one of/,
  ],
  [
    "a missing translation",
    { ...valid, entries: [{ ...entry, "en-US": undefined }] },
    /missing en-US/,
  ],
  [
    "a blank translation",
    { ...valid, entries: [{ ...entry, "zh-CN": " " }] },
    /zh-CN must be non-empty/,
  ],
  [
    "an over-long text",
    {
      ...valid,
      entries: [
        { ...entry, "zh-CN": "字".repeat(MAX_RELEASE_NOTE_TEXT_LENGTH + 1) },
      ],
    },
    /exceeds/,
  ],
  ...[
    "\n",
    "\u0000",
    "\u007f",
    "\u061C",
    "\u200F",
    "\u2028",
    "\u2029",
    "\u202E",
    "\u2066",
  ].map((character): [string, unknown, RegExp] => [
    `the character U+${character.charCodeAt(0).toString(16).padStart(4, "0")}`,
    { ...valid, entries: [{ ...entry, "en-US": `a${character}b` }] },
    /control, line separator, or bidirectional/,
  ]),
];
for (const [label, content, expected] of invalidNotes) {
  Deno.test(`notes with ${label} are rejected`, () => {
    assert.throws(
      () =>
        parseReleaseNote(
          "0.7.0.json",
          typeof content === "string" ? content : JSON.stringify(content),
        ),
      expected,
    );
  });
}

Deno.test("notes that are not UTF-8 are rejected", () =>
  withTempDirectory(async (root) => {
    await writeNotes(root, {});
    const [before, after] = JSON.stringify({
      ...releaseNote("0.7.0"),
      entries: [{ ...entry, "en-US": "split here" }],
    }).split("split");
    await Deno.writeFile(
      path.join(root, "release-notes", "0.7.0.json"),
      new Uint8Array([
        ...new TextEncoder().encode(before),
        0xff,
        ...new TextEncoder().encode(after),
      ]),
    );
    await assert.rejects(
      readReleaseNotes(root),
      /release-notes\/0\.7\.0\.json: must be valid UTF-8/,
    );
  }),
);

Deno.test("a build without a notes directory has no notes", () =>
  withTempDirectory(async (root) => {
    assert.deepEqual(await readReleaseNotes(root), []);
  }),
);

for (const version of ["0.7.0", "0.7.0-rc.1", "0.7.0+yellow", "0.7.0-rc.1+x"]) {
  Deno.test(`packaging ${version} requires the notes of 0.7.0`, () =>
    withTempDirectory(async (root) => {
      await writeNotes(root, { "0.6.9.json": releaseNote("0.6.9") });
      await commitAll(root);
      await assert.rejects(
        requireReleaseNotes(root, version),
        /requires release-notes\/0\.7\.0\.json/,
      );
      await writeNotes(root, { "0.7.0.json": releaseNote("0.7.0") });
      await commitAll(root);
      await requireReleaseNotes(root, version);
    }),
  );
}

Deno.test("one invalid historical file blocks every versioned package", () =>
  withTempDirectory(async (root) => {
    await writeNotes(root, {
      "0.7.0.json": releaseNote("0.7.0"),
      "0.6.9.json": "{}",
    });
    await commitAll(root);
    await assert.rejects(
      requireReleaseNotes(root, "0.7.0"),
      /release-notes\/0\.6\.9\.json: the document is missing/,
    );
  }),
);

const uncommittedChanges: [string, (root: string) => Promise<void>][] = [
  [
    "a modified file",
    (root) =>
      writeNotes(root, {
        "0.7.0.json": { ...releaseNote("0.7.0"), date: "2026-12-11" },
      }),
  ],
  [
    "a staged file",
    async (root) => {
      await writeNotes(root, { "0.8.0.json": releaseNote("0.8.0") });
      await git(root, ["add", "--all"]);
    },
  ],
  [
    "a deleted file",
    (root) => Deno.remove(path.join(root, "release-notes", "0.6.9.json")),
  ],
  [
    "an untracked file",
    (root) => writeNotes(root, { "0.8.0.json": releaseNote("0.8.0") }),
  ],
  [
    "an ignored file",
    (root) => writeNotes(root, { "0.9.0.json": releaseNote("0.9.0") }),
  ],
];
for (const [label, change] of uncommittedChanges) {
  Deno.test(`packaging rejects ${label} under release-notes`, () =>
    withTempDirectory(async (root) => {
      await Deno.writeTextFile(path.join(root, ".gitignore"), "0.9.0.json\n");
      await writeNotes(root, {
        "0.6.9.json": releaseNote("0.6.9"),
        "0.7.0.json": releaseNote("0.7.0"),
      });
      await commitAll(root);
      await change(root);
      await assert.rejects(
        requireReleaseNotes(root, "0.7.0"),
        /differs from the checked-out commit/,
      );
    }),
  );
}

Deno.test("edits outside release-notes do not block a patched package", () =>
  withTempDirectory(async (root) => {
    await writeNotes(root, { "0.7.0.json": releaseNote("0.7.0") });
    await Deno.writeTextFile(path.join(root, "seed.sql"), "");
    await commitAll(root);
    await Deno.writeTextFile(path.join(root, "seed.sql"), "patched");
    await requireReleaseNotes(root, "0.7.0");
  }),
);

for (const flag of ["--assume-unchanged", "--skip-worktree"]) {
  Deno.test(`packaging sees edits that ${flag} hides from git status`, () =>
    withTempDirectory(async (root) => {
      await writeNotes(root, { "0.7.0.json": releaseNote("0.7.0") });
      await commitAll(root);
      await git(root, ["update-index", flag, "release-notes/0.7.0.json"]);
      await writeNotes(root, {
        "0.7.0.json": { ...releaseNote("0.7.0"), date: "2026-12-11" },
      });
      await assert.rejects(
        requireReleaseNotes(root, "0.7.0"),
        /differs from the checked-out commit/,
      );
    }),
  );
}

Deno.test("packaging rejects an edit that only remains staged", () =>
  withTempDirectory(async (root) => {
    await writeNotes(root, { "0.7.0.json": releaseNote("0.7.0") });
    await commitAll(root);
    await writeNotes(root, {
      "0.7.0.json": { ...releaseNote("0.7.0"), date: "2026-12-11" },
    });
    await git(root, ["add", "--all"]);
    await writeNotes(root, { "0.7.0.json": releaseNote("0.7.0") });
    await assert.rejects(
      requireReleaseNotes(root, "0.7.0"),
      /differs from the checked-out commit/,
    );
  }),
);

Deno.test("the packaging check leaves no trace in the checkout", () =>
  withTempDirectory(async (root) => {
    await writeNotes(root, { "0.7.0.json": releaseNote("0.7.0") });
    await commitAll(root);
    await git(root, ["config", "core.splitIndex", "true"]);
    const hook = path.join(root, ".git", "hooks", "post-index-change");
    await Deno.mkdir(path.dirname(hook), { recursive: true });
    await Deno.writeTextFile(
      hook,
      '#!/bin/sh\ntouch "$(git rev-parse --show-toplevel)/.git/hook-ran"\n',
      { mode: 0o755 },
    );
    const entries = () =>
      [...Deno.readDirSync(path.join(root, ".git"))]
        .map((entry) => entry.name)
        .sort();
    const before = entries();
    await requireReleaseNotes(root, "0.7.0");
    assert.deepEqual(entries(), before);
  }),
);

Deno.test("a source copy inside another repository cannot be packaged", () =>
  withTempDirectory(async (root) => {
    const copy = path.join(root, "copy");
    await writeNotes(copy, { "0.7.0.json": releaseNote("0.7.0") });
    await commitAll(root);
    await assert.rejects(
      requireReleaseNotes(copy, "0.7.0"),
      /requires a Git checkout/,
    );
  }),
);

Deno.test("notes outside a Git checkout cannot be packaged", () =>
  withTempDirectory(async (root) => {
    await writeNotes(root, { "0.7.0.json": releaseNote("0.7.0") });
    await assert.rejects(
      requireReleaseNotes(root, "0.7.0"),
      /requires a Git checkout/,
    );
  }),
);

Deno.test({
  name: "linked notes are rejected because Git does not track their content",
  // Creating links needs extra privileges on Windows.
  ignore: Deno.build.os === "windows",
  fn: () =>
    withTempDirectory(async (root) => {
      await writeNotes(root, {});
      const outside = path.join(root, "outside.json");
      await Deno.writeTextFile(outside, JSON.stringify(releaseNote("0.7.0")));
      await Deno.symlink(
        outside,
        path.join(root, "release-notes", "0.7.0.json"),
      );
      await commitAll(root);
      await assert.rejects(
        requireReleaseNotes(root, "0.7.0"),
        /must be a regular file/,
      );
    }),
});
