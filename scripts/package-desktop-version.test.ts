import assert from "node:assert/strict";
import { replaceCargoPackageVersion } from "./package-desktop-version.ts";

Deno.test("version replacement only changes the package version", () => {
  assert.equal(
    replaceCargoPackageVersion(
      '[package]\nname = "ora"\nversion = "0.1.0"\n[dependencies]\nx = { version = "2" }\n',
      "1.2.3-beta.1",
    ),
    '[package]\nname = "ora"\nversion = "1.2.3-beta.1"\n[dependencies]\nx = { version = "2" }\n',
  );
});

Deno.test("version replacement rejects a missing package version", () => {
  assert.throws(
    () => replaceCargoPackageVersion('[workspace]\nversion = "1.0.0"', "2.0.0"),
    /package version/,
  );
});
