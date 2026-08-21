import { describe, expect, it } from "vitest";
import {
  ancestorDirectories,
  buildComposerFileActions,
  fileBasename,
  fileParentPath,
  filterComposerActions,
  MAX_COMPOSER_FILE_ACTIONS,
  mentionEntriesFromDirectoryListing,
  mentionEntriesFromFileSearch,
  rankComposerMentionEntries,
  mentionRelevanceScore,
  takeComposerFilePaths,
  visibleComposerActions,
} from "./composer-actions";

describe("takeComposerFilePaths", () => {
  it("stops after the menu cap without requiring a full array first", () => {
    function* infinitePaths() {
      let index = 0;
      for (;;) {
        yield `src/file-${index}.ts`;
        index += 1;
      }
    }
    expect(takeComposerFilePaths(infinitePaths())).toEqual(
      Array.from(
        { length: MAX_COMPOSER_FILE_ACTIONS },
        (_, index) => `src/file-${index}.ts`,
      ),
    );
  });
});

describe("mentionEntriesFromDirectoryListing", () => {
  it("ranks empty-query hits as directories first, then A–Z by basename", () => {
    expect(
      rankComposerMentionEntries(
        mentionEntriesFromDirectoryListing([
          { path: "README.md", kind: "file" },
          { path: "src", kind: "directory" },
          { path: "packages", kind: "directory" },
          { path: "app.ts", kind: "file" },
        ]),
        "",
      ),
    ).toEqual([
      { path: "packages", kind: "directory" },
      { path: "src", kind: "directory" },
      { path: "app.ts", kind: "file" },
      { path: "README.md", kind: "file" },
    ]);
  });
});

describe("mentionRelevanceScore", () => {
  it("matches Windows-style queries against slash-normalized paths", () => {
    expect(
      mentionRelevanceScore(
        { path: "src/utils/index.ts", kind: "file" },
        "src\\utils",
      ),
    ).toBeGreaterThan(0);
  });
});

describe("rankComposerMentionEntries", () => {
  it("prefers basename matches over weak path hits, without burying files under every folder", () => {
    expect(
      rankComposerMentionEntries(
        [
          { path: "vendor/legacy-app/readme.md", kind: "file" },
          { path: "packages", kind: "directory" },
          { path: "src/app.ts", kind: "file" },
          { path: "packages/app-shell", kind: "directory" },
        ],
        "app",
      ).map((entry) => entry.path),
    ).toEqual([
      "packages/app-shell",
      "src/app.ts",
      "vendor/legacy-app/readme.md",
      "packages",
    ]);
  });

  it("still surfaces a late strong basename match beyond the old early pool", () => {
    const earlyNoise = Array.from({ length: 250 }, (_, index) => ({
      path: `noise/file-${index}.ts`,
      kind: "file" as const,
    }));
    expect(
      rankComposerMentionEntries(
        [...earlyNoise, { path: "src/app.ts", kind: "file" }],
        "app",
      )[0],
    ).toEqual({ path: "src/app.ts", kind: "file" });
  });
});

describe("mentionEntriesFromFileSearch", () => {
  it("includes matching ancestor directories and ranks them with files", () => {
    expect(
      rankComposerMentionEntries(
        mentionEntriesFromFileSearch(
          [
            { kind: "file", path: "packages/ui/button.tsx" },
            { kind: "file", path: "packages/app-shell/composer.tsx" },
          ],
          "pack",
        ),
        "pack",
      ),
    ).toEqual([
      { path: "packages", kind: "directory" },
      { path: "packages/app-shell", kind: "directory" },
      { path: "packages/ui", kind: "directory" },
      { path: "packages/ui/button.tsx", kind: "file" },
      { path: "packages/app-shell/composer.tsx", kind: "file" },
    ]);
  });

  it("omits ancestors that do not match the query", () => {
    expect([
      ...mentionEntriesFromFileSearch(
        [{ kind: "file", path: "src/app.ts" }],
        "app",
      ),
    ]).toEqual([{ path: "src/app.ts", kind: "file" }]);
  });
});

describe("ancestorDirectories", () => {
  it("returns root-first parents", () => {
    expect(ancestorDirectories("a/b/c.ts")).toEqual(["a", "a/b"]);
    expect(ancestorDirectories("c.ts")).toEqual([]);
  });
});

describe("buildComposerFileActions", () => {
  it("uses basename labels and parent-path descriptions", () => {
    expect(
      buildComposerFileActions([
        { path: "src/app.ts", kind: "file" },
        { path: "README.md", kind: "file" },
        { path: "src", kind: "directory" },
      ]),
    ).toEqual([
      {
        id: "file:src/app.ts",
        group: "files",
        label: "app.ts",
        description: "src",
        path: "src/app.ts",
        entryKind: "file",
      },
      {
        id: "file:README.md",
        group: "files",
        label: "README.md",
        description: ".",
        path: "README.md",
        entryKind: "file",
      },
      {
        id: "file:src",
        group: "files",
        label: "src",
        description: ".",
        path: "src",
        entryKind: "directory",
      },
    ]);
  });

  it("dedupes paths and caps the action list", () => {
    const entries = Array.from(
      { length: MAX_COMPOSER_FILE_ACTIONS + 5 },
      (_, index) => ({
        path: `pkg/file-${index}.ts`,
        kind: "file" as const,
      }),
    );
    entries.push({ path: "pkg/file-0.ts", kind: "file" });
    const actions = buildComposerFileActions(entries);
    expect(actions).toHaveLength(MAX_COMPOSER_FILE_ACTIONS);
    expect(
      new Set(
        actions.flatMap((action) =>
          action.group === "files" ? [action.path] : [],
        ),
      ).size,
    ).toBe(MAX_COMPOSER_FILE_ACTIONS);
  });
});

describe("file path helpers", () => {
  it("splits basenames and parents across separators", () => {
    expect(fileBasename("a/b\\c.ts")).toBe("c.ts");
    expect(fileParentPath("a/b/c.ts")).toBe("a/b");
    expect(fileParentPath("c.ts")).toBe(".");
  });
});

describe("filterComposerActions", () => {
  it("matches file actions by full path as well as label", () => {
    const actions = buildComposerFileActions([
      { path: "packages/app-shell/composer.tsx", kind: "file" },
      { path: "apps/desktop/main.rs", kind: "file" },
    ]);
    expect(
      filterComposerActions(actions, "app-shell").flatMap((action) =>
        action.group === "files" ? [action.path] : [],
      ),
    ).toEqual(["packages/app-shell/composer.tsx"]);
  });
});

describe("visibleComposerActions", () => {
  it("does not collapse the files group", () => {
    const files = buildComposerFileActions(
      Array.from({ length: 12 }, (_, index) => ({
        path: `src/file-${index}.ts`,
        kind: "file" as const,
      })),
    );
    expect(visibleComposerActions(files, new Set())).toHaveLength(12);
  });
});
