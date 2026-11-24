import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import {
  MAX_RELEASE_NOTE_TEXT_LENGTH,
  parseReleaseNote,
  readReleaseNotes,
} from "./release-notes.ts";
import { releaseNote, withTempDirectory } from "./test-support.ts";

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
  ...["\n", "\u0000", "\u007f", "\u202E", "\u2066"].map(
    (character): [string, unknown, RegExp] => [
      `the character U+${character.charCodeAt(0).toString(16).padStart(4, "0")}`,
      { ...valid, entries: [{ ...entry, "en-US": `a${character}b` }] },
      /control or bidirectional/,
    ],
  ),
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

Deno.test("a build without a notes directory has no notes", () =>
  withTempDirectory(async (root) => {
    assert.deepEqual(await readReleaseNotes(root), []);
  }),
);
