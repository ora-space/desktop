import assert from "node:assert/strict";
import path from "node:path";
import { createUpdateManifest } from "./create-desktop-update-manifest.ts";
import { withTempDirectory } from "./test-support.ts";

/** Creates signed assets in nested download directories like the release workflow. */
async function signedAssets(root: string): Promise<void> {
  for (const [platform, name] of [
    ["windows", "Ora_1.2.3_x64-setup.exe"],
    ["mac", "Ora.app.tar.gz"],
    ["linux", "Ora 1.2.3.AppImage"],
  ]) {
    const file = path.join(root, platform, name);
    await Deno.mkdir(path.dirname(file), { recursive: true });
    await Deno.writeTextFile(file, "artifact");
    await Deno.writeTextFile(`${file}.sig`, `signature-${platform}\n`);
  }
}

Deno.test(
  "updater manifest contains complete signed targets and escaped download URLs",
  () =>
    withTempDirectory(async (root) => {
      await signedAssets(root);
      const manifest = await createUpdateManifest({
        bundlesDirectory: root,
        releaseTag: "v1.2.3+build",
        now: () => new Date("2026-09-05T02:00:00Z"),
      });
      const base =
        "https://github.com/ora-space/desktop/releases/download/v1.2.3%2Bbuild/";
      assert.deepEqual(manifest, {
        version: "1.2.3+build",
        notes: "Ora v1.2.3+build",
        pub_date: "2026-09-05T02:00:00.000Z",
        platforms: {
          "windows-x86_64": {
            url: `${base}Ora_1.2.3_x64-setup.exe`,
            signature: "signature-windows\n",
          },
          "darwin-aarch64": {
            url: `${base}Ora.app.tar.gz`,
            signature: "signature-mac\n",
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
    await Deno.remove(path.join(root, "mac", "Ora.app.tar.gz.sig"));
    await Deno.writeTextFile(path.join(root, "latest.json"), "previous");
    await assert.rejects(
      createUpdateManifest({ bundlesDirectory: root, releaseTag: "v1.2.3" }),
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
      path.join(root, "duplicate", "Ora.app.tar.gz"),
      "duplicate",
    );
    await assert.rejects(
      createUpdateManifest({ bundlesDirectory: root, releaseTag: "v1" }),
      /Duplicate updater asset/,
    );
  }),
);
