import assert from "node:assert/strict";
import path from "node:path";
import { createUpdateManifest } from "./create-desktop-update-manifest.ts";
import { releaseNote, withTempDirectory } from "./test-support.ts";

/** Creates signed assets in nested download directories like the release workflow. */
async function signedAssets(root: string): Promise<void> {
  for (const [platform, name] of [
    ["windows", "Ora_1.2.3_x64-setup.exe"],
    ["linux", "Ora 1.2.3.AppImage"],
  ]) {
    const file = path.join(root, platform, name);
    await Deno.mkdir(path.dirname(file), { recursive: true });
    await Deno.writeTextFile(file, "artifact");
    await Deno.writeTextFile(`${file}.sig`, `signature-${platform}\n`);
  }
}

/** Writes the notes of the checkout that a release job generates its manifest from. */
async function writeNotes(
  root: string,
  files: Record<string, unknown>,
): Promise<void> {
  await Deno.mkdir(path.join(root, "release-notes"));
  for (const [name, content] of Object.entries(files)) {
    await Deno.writeTextFile(
      path.join(root, "release-notes", name),
      typeof content === "string" ? content : JSON.stringify(content),
    );
  }
}

/** Projects a notes file to what a manifest carries of it. */
function manifestNote(version: string) {
  const { date, entries } = releaseNote(version);
  return { version, date, entries };
}

Deno.test(
  "updater manifest contains complete signed targets and escaped download URLs",
  () =>
    withTempDirectory(async (root) => {
      await signedAssets(root);
      await writeNotes(root, { "1.2.3.json": releaseNote("1.2.3") });
      const manifest = await createUpdateManifest({
        bundlesDirectory: root,
        root,
        releaseTag: "v1.2.3+build",
        now: () => new Date("2026-09-05T02:00:00Z"),
      });
      const base =
        "https://github.com/ora-space/desktop/releases/download/v1.2.3%2Bbuild/";
      assert.deepEqual(manifest, {
        version: "1.2.3+build",
        notes: "Ora v1.2.3+build",
        pub_date: "2026-09-05T02:00:00.000Z",
        ora_release_notes: { schema: 1, versions: [manifestNote("1.2.3")] },
        platforms: {
          "windows-x86_64": {
            url: `${base}Ora_1.2.3_x64-setup.exe`,
            signature: "signature-windows\n",
          },
          "linux-x86_64": {
            url: `${base}Ora%201.2.3.AppImage`,
            signature: "signature-linux\n",
          },
        },
      });
      assert.deepEqual(
        JSON.parse(await Deno.readTextFile(path.join(root, "latest.json"))),
        manifest,
      );
    }),
);

Deno.test("missing signatures do not overwrite an existing manifest", () =>
  withTempDirectory(async (root) => {
    await signedAssets(root);
    await writeNotes(root, { "1.2.3.json": releaseNote("1.2.3") });
    await Deno.remove(path.join(root, "linux", "Ora 1.2.3.AppImage.sig"));
    await Deno.writeTextFile(path.join(root, "latest.json"), "previous");
    await assert.rejects(
      createUpdateManifest({
        bundlesDirectory: root,
        root,
        releaseTag: "v1.2.3",
      }),
      /Expected one signed/,
    );
    assert.equal(
      await Deno.readTextFile(path.join(root, "latest.json")),
      "previous",
    );
  }),
);

Deno.test("updater rejects missing tags and ambiguous assets", () =>
  withTempDirectory(async (root) => {
    await assert.rejects(
      createUpdateManifest({ bundlesDirectory: root, releaseTag: undefined }),
      /RELEASE_TAG/,
    );
    await signedAssets(root);
    await Deno.mkdir(path.join(root, "duplicate"));
    await Deno.writeTextFile(
      path.join(root, "duplicate", "Ora 1.2.3.AppImage"),
      "duplicate",
    );
    await assert.rejects(
      createUpdateManifest({ bundlesDirectory: root, releaseTag: "v1" }),
      /Duplicate updater asset/,
    );
  }),
);

Deno.test(
  "updater manifest carries the latest ten versions up to the released one",
  () =>
    withTempDirectory(async (root) => {
      await signedAssets(path.join(root, "bundles"));
      await writeNotes(
        root,
        Object.fromEntries(
          Array.from({ length: 12 }, (_, minor) => `1.${minor}.0`).map(
            (version) => [`${version}.json`, releaseNote(version)],
          ),
        ),
      );
      const manifest = await createUpdateManifest({
        bundlesDirectory: path.join(root, "bundles"),
        root,
        releaseTag: "v1.10.0-rc.1",
      });
      assert.equal(manifest.notes, "Ora v1.10.0-rc.1");
      assert.deepEqual(manifest.ora_release_notes, {
        schema: 1,
        versions: [10, 9, 8, 7, 6, 5, 4, 3, 2, 1].map((minor) =>
          manifestNote(`1.${minor}.0`),
        ),
      });
    }),
);

for (const [label, files, expected] of [
  [
    "the released version has no notes",
    { "1.0.0.json": releaseNote("1.0.0") },
    /requires release-notes\/1\.1\.0\.json/,
  ],
  [
    // serde_json refuses a lone surrogate anywhere, so clients would lose the whole manifest.
    "a text holds a lone surrogate",
    {
      "1.1.0.json": {
        ...releaseNote("1.1.0"),
        entries: [{ category: "fix", "zh-CN": "a\ud800b", "en-US": "Fixed" }],
      },
    },
    /1\.1\.0\.json: entries\[0\]\.zh-CN/,
  ],
  [
    "a historical file is invalid",
    { "1.0.0.json": "broken", "1.1.0.json": releaseNote("1.1.0") },
    /1\.0\.0\.json: invalid JSON/,
  ],
] as const) {
  Deno.test(`no updater manifest is written when ${label}`, () =>
    withTempDirectory(async (root) => {
      const bundles = path.join(root, "bundles");
      await signedAssets(bundles);
      await writeNotes(root, files);
      await assert.rejects(
        createUpdateManifest({
          bundlesDirectory: bundles,
          root,
          releaseTag: "v1.1.0",
        }),
        expected,
      );
      await assert.rejects(Deno.stat(path.join(bundles, "latest.json")));
    }),
  );
}

// The Desktop host parses this very file, so a change of the envelope cannot pass on one side only.
Deno.test("updater manifest matches the fixture the Desktop host reads", () =>
  withTempDirectory(async (root) => {
    await signedAssets(path.join(root, "bundles"));
    await writeNotes(
      root,
      Object.fromEntries(
        Array.from({ length: 12 }, (_, minor) => `1.${minor}.0`).map(
          (version) => [`${version}.json`, releaseNote(version)],
        ),
      ),
    );
    const manifest = await createUpdateManifest({
      bundlesDirectory: path.join(root, "bundles"),
      root,
      releaseTag: "v1.10.0",
      now: () => new Date("2026-12-10T00:00:00Z"),
    });
    const fixture = new URL("./update-manifest-fixture.json", import.meta.url);
    // Rewrite it with UPDATE_FIXTURES=1 after a deliberate change, then run the Desktop tests.
    if (Deno.env.get("UPDATE_FIXTURES") === "1") {
      await Deno.writeTextFile(
        fixture,
        `${JSON.stringify(manifest, null, 2)}\n`,
      );
    }
    assert.deepEqual(manifest, JSON.parse(await Deno.readTextFile(fixture)));
  }),
);
