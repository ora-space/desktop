import assert from "node:assert/strict";
import path from "node:path";
import { checkFeatureInterfaces } from "./check-feature-interfaces.ts";
import { withTempDirectory } from "./test-support.ts";

/** Builds a tiny real source tree so alias resolution and exported symbols use TypeScript itself. */
async function fixture(
  root: string,
): Promise<(filename: string, source: string) => Promise<void>> {
  const write = async (filename: string, source: string) => {
    const target = path.join(root, filename);
    await Deno.mkdir(path.dirname(target), { recursive: true });
    await Deno.writeTextFile(target, source);
  };
  await Deno.mkdir(path.join(root, "apps"));
  await write(
    "packages/app-shell/tsconfig.json",
    JSON.stringify({
      compilerOptions: {
        module: "ESNext",
        moduleResolution: "Bundler",
        paths: { "@alpha/*": ["./src/features/alpha/*"] },
      },
      include: ["src"],
    }),
  );
  await write(
    "packages/app-shell/src/features/alpha/public.ts",
    "export const Public = 1; export const Secret = 2; export interface PublicType { value: number }\n",
  );
  await write(
    "packages/app-shell/src/features/alpha/private.ts",
    "export const Hidden = 3;\n",
  );
  await write(
    "packages/app-shell/src/features/alpha/translations.ts",
    "export const translations = {};\n",
  );
  await write(
    "packages/app-shell/src/features/alpha/interface.json",
    JSON.stringify({
      owner: "Alpha feature",
      modules: {
        "./public": {
          exports: ["Public", "PublicType"],
          purpose: "Consumers use Alpha through these two named exports.",
        },
        "./translations": {
          exports: ["translations"],
          purpose: "Pure resources belong to the single i18n composition.",
          kind: "resources",
        },
      },
    }),
  );
  await write(
    "packages/app-shell/src/features/beta/consumer.ts",
    "export {};\n",
  );
  return write;
}

/** Removes only the source position, keeping each diagnostic's exact policy explanation. */
function messages(root: string): string[] {
  return checkFeatureInterfaces(root)
    .map((message) => message.slice(message.indexOf(": ") + 2))
    .sort();
}

Deno.test(
  "feature interfaces allow named public access, aliases and same-owner private implementation",
  () =>
    withTempDirectory(async (root) => {
      const write = await fixture(root);
      await write(
        "packages/app-shell/src/features/beta/consumer.ts",
        `
      import { Public as renamed, type PublicType } from "@alpha/public";
      export { Public as Forwarded } from "../alpha/public";
      type Imported = import("../alpha/public").PublicType;
      // import { Hidden } from "../alpha/private";
      const explanation = 'require("../alpha/private")';
      vi.mock("../alpha/public", () => ({ Public: 4 }));
    `,
      );
      await write(
        "packages/app-shell/src/features/alpha/local.ts",
        'import { Hidden } from "./private"; import * as all from "./public";',
      );
      await write(
        "apps/main.ts",
        'import { Public } from "../packages/app-shell/src/features/alpha/public";',
      );
      assert.deepEqual(checkFeatureInterfaces(root), []);
    }),
);

Deno.test(
  "feature interfaces reject private modules and private symbols through static, aliased and type access",
  () =>
    withTempDirectory(async (root) => {
      const write = await fixture(root);
      await write(
        "packages/app-shell/src/features/beta/consumer.ts",
        `
      import { Hidden } from "../alpha/private";
      import { Secret as Alias } from "@alpha/public";
      export { Secret as Forwarded } from "../alpha/public";
      type Imported = import("../alpha/public").Secret;
      vi.mock("../alpha/public", () => ({ Secret: 4 }));
    `,
      );
      assert.deepEqual(
        messages(root),
        [
          "private feature module ../alpha/private",
          "private feature export Secret from @alpha/public",
          "private feature export Secret from ../alpha/public",
          "private feature export Secret from ../alpha/public",
          "private feature export Secret from ../alpha/public",
        ].sort(),
      );
    }),
);

Deno.test(
  "whole-module and nonliteral imports cannot bypass the named interface",
  () =>
    withTempDirectory(async (root) => {
      const write = await fixture(root);
      await write(
        "packages/app-shell/src/features/beta/consumer.ts",
        `
      import * as all from "../alpha/public";
      export * from "../alpha/public";
      const lazy = import("../alpha/public");
      const required = require("../alpha/public");
      import legacy = require("../alpha/public");
      type Whole = typeof import("../alpha/public");
      vi.mock("../alpha/public", (original) => ({ Public: original() }));
      vi.mock("../alpha/public", () => ({ ...anything }));
      const unchecked = import(computed);
    `,
      );
      assert.deepEqual(messages(root), [
        ...Array<string>(8).fill(
          "cross-feature access requires named imports, not whole-module access (../alpha/public)",
        ),
        "module access must use a static string",
      ]);
    }),
);

Deno.test(
  "shared state cannot depend on public UI and resources have one composition owner",
  () =>
    withTempDirectory(async (root) => {
      const write = await fixture(root);
      await write(
        "packages/app-shell/src/state/data/model.ts",
        'import { Public } from "../../features/alpha/public";',
      );
      await write(
        "packages/app-shell/src/i18n/resources.ts",
        'import { translations } from "../features/alpha/translations";',
      );
      await write(
        "packages/app-shell/src/features/beta/consumer.ts",
        'import { translations } from "../alpha/translations";',
      );
      assert.deepEqual(messages(root), [
        "feature resources belong to the single i18n composition",
        "shared state must not depend on feature UI (../../features/alpha/public)",
      ]);
    }),
);

Deno.test(
  "stale public exports and wildcard policies fail instead of silently broadening access",
  () =>
    withTempDirectory(async (root) => {
      const write = await fixture(root);
      await write(
        "packages/app-shell/src/features/alpha/interface.json",
        JSON.stringify({
          owner: "Alpha feature",
          modules: {
            "./public": {
              exports: ["Deleted"],
              purpose:
                "A stale declaration must be removed with its implementation.",
            },
            "./private": {
              exports: ["*"],
              purpose:
                "Wildcard access cannot declare a narrow feature interface.",
            },
          },
        }),
      );
      assert.deepEqual(messages(root), [
        "./public no longer exports Deleted",
        "invalid public module ./private; require local named exports and a meaningful purpose",
      ]);
    }),
);

Deno.test(
  "features without a manifest are private and directory imports resolve to the real index",
  () =>
    withTempDirectory(async (root) => {
      const write = await fixture(root);
      await write(
        "packages/app-shell/src/features/gamma/index.ts",
        "export const Hidden = 4;",
      );
      await write(
        "packages/app-shell/src/features/beta/consumer.ts",
        'import { Hidden } from "../gamma";',
      );
      assert.deepEqual(messages(root), ["private feature module ../gamma"]);
    }),
);
