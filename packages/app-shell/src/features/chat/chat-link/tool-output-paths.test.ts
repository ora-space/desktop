import { describe, expect, it } from "vitest";
import type { ChatToolCall } from "@ora/chat";
import {
  collectToolOutputArtifacts,
  collectToolOutputPaths,
  extractArtifactDirectoriesFromText,
  extractArtifactPathsFromText,
  isPlainPathList,
  stripListMarker,
} from "./tool-output-paths";

describe("extractArtifactPathsFromText", () => {
  it("keeps concrete file lines and drops commands and globs", () => {
    expect(
      extractArtifactPathsFromText(
        "D:\\project\\desktop\\README.md\n**/*.md\ncargo test\n- docs/guide.md",
      ),
    ).toEqual(["D:\\project\\desktop\\README.md", "docs/guide.md"]);
    expect(
      extractArtifactPathsFromText("file:///D:/project/desktop/docs/guide.md"),
    ).toEqual(["D:/project/desktop/docs/guide.md"]);
  });

  it("extracts paths from ripgrep-style locations without result text", () => {
    expect(
      extractArtifactPathsFromText(
        "src/main.rs:12:fn main() {}\nD:\\repo\\src\\lib.rs:4:9:pub fn run() {}",
      ),
    ).toEqual(["src/main.rs", "D:\\repo\\src\\lib.rs"]);
  });
});

describe("extractArtifactDirectoriesFromText", () => {
  it("accepts absolute and slash-terminated directories without treating files as directories", () => {
    expect(
      extractArtifactDirectoriesFromText(
        "C:\\Users\\zhans\\projects\\hapi\\cli\n.git/\ndocs/\nREADME.md",
      ),
    ).toEqual([".git", "docs"]);
  });

  it("keeps bare listing entries unresolved instead of guessing their type", () => {
    const output = ".git\n.github\ncli\ndocs\nLICENSE\nAGENTS.md\npackage.json";
    const tool: ChatToolCall = {
      kind: "toolCall",
      id: "list",
      title: "Get-ChildItem -Name",
      toolKind: "execute",
      status: "completed",
      content: [{ type: "content", content: { type: "text", text: output } }],
      locations: [],
      createdAt: 1,
      updatedAt: 1,
    };
    expect(collectToolOutputArtifacts(tool)).toEqual({
      files: [],
      directories: [],
      unknown: [
        ".git",
        ".github",
        "cli",
        "docs",
        "LICENSE",
        "AGENTS.md",
        "package.json",
      ],
    });
  });

  it("does not infer status output as workspace artifacts", () => {
    const tool: ChatToolCall = {
      kind: "toolCall",
      id: "test",
      title: "Run tests",
      toolKind: "execute",
      status: "completed",
      content: [
        { type: "content", content: { type: "text", text: "PASS\nFAIL" } },
      ],
      locations: [],
      createdAt: 1,
      updatedAt: 1,
    };
    expect(collectToolOutputArtifacts(tool)).toEqual({
      files: [],
      directories: [],
      unknown: [],
    });
  });

  it("ignores URLs and non-path diagnostic metadata in raw output", () => {
    const tool: ChatToolCall = {
      kind: "toolCall",
      id: "metadata",
      title: "Fetch metadata",
      status: "completed",
      content: [],
      locations: [],
      rawOutput: {
        error: "src/missing.rs",
        homepage: "https://example.com/report.pdf",
        result: { message: "docs/failure.md" },
      },
      createdAt: 1,
      updatedAt: 1,
    };
    expect(collectToolOutputArtifacts(tool)).toEqual({
      files: [],
      directories: [],
      unknown: [],
    });
  });

  it("uses PowerShell Mode rows as explicit file and directory types", () => {
    const output =
      "Mode   Name\n----   ----\nd----- cli\n-a---- LICENSE\n-a---- package.json";
    expect(extractArtifactDirectoriesFromText(output)).toEqual(["cli"]);
    expect(extractArtifactPathsFromText(output)).toEqual([
      "LICENSE",
      "package.json",
    ]);
  });

  it("supports PowerShell Name Mode rows and extensionless files", () => {
    const output =
      "Name       Mode\n----       ----\n.codex     d-----\npackages   d-----\ninstall    -a----\nLICENSE    -a----";
    expect(extractArtifactDirectoriesFromText(output)).toEqual([
      ".codex",
      "packages",
    ]);
    expect(extractArtifactPathsFromText(output)).toEqual([
      "install",
      "LICENSE",
    ]);
  });

  it("uses aligned PowerShell default table columns instead of dates as paths", () => {
    const output =
      "Mode   LastWriteTime       Length Name\nd-----  8/24/2026 10:00 AM         docs\n-a----  8/24/2026 10:01 AM   1200  LICENSE";
    expect(extractArtifactDirectoriesFromText(output)).toEqual(["docs"]);
    expect(extractArtifactPathsFromText(output)).toEqual(["LICENSE"]);
  });

  it("ignores ANSI styling when locating PowerShell table columns", () => {
    const output =
      "\u001b[32;1mMode \u001b[0m\u001b[32;1m Length\u001b[0m\u001b[32;1m Name\u001b[0m\n" +
      "\u001b[32;1m---- \u001b[0m \u001b[32;1m------\u001b[0m \u001b[32;1m----\u001b[0m\n" +
      "d----        packages\n-a--- 14150  install\n-a--- 1086   LICENSE";
    expect(extractArtifactDirectoriesFromText(output)).toEqual(["packages"]);
    expect(extractArtifactPathsFromText(output)).toEqual([
      "install",
      "LICENSE",
    ]);
  });

  it("reads a Name PSIsContainer listing as explicit directories and files", () => {
    const esc = String.fromCharCode(27);
    const output = [
      "",
      `${esc}[32;1mName           ${esc}[0m${esc}[32;1m PSIsContainer${esc}[0m`,
      `${esc}[32;1m----           ${esc}[0m ${esc}[32;1m-------------${esc}[0m`,
      ".git                     True",
      "docs                     True",
      "main.py                 False",
      "README.md               False",
      "",
    ].join(String.fromCharCode(13, 10));
    const tool: ChatToolCall = {
      kind: "toolCall",
      id: "ps-container",
      title: "Get-ChildItem -Force | Select-Object Name, PSIsContainer",
      toolKind: "execute",
      status: "completed",
      content: [{ type: "content", content: { type: "text", text: output } }],
      locations: [],
      rawInput: {
        command: "Get-ChildItem -Force | Select-Object Name, PSIsContainer",
      },
      createdAt: 1,
      updatedAt: 1,
    };
    // The header rule (`---- -------------`) must not become a file, and it must
    // not count as typed evidence that suppresses the rest of the parse.
    expect(collectToolOutputArtifacts(tool)).toEqual({
      files: ["main.py", "README.md"],
      directories: [".git", "docs"],
      unknown: [],
    });
  });

  it("indexes nested relative entries from a recursive listing", () => {
    const output = [
      ".claude",
      String.raw`.claude\commands`,
      String.raw`.claude\commands\opsx`,
      "docs",
      String.raw`docs\superpowers`,
      "main.py",
    ].join(String.fromCharCode(10));
    const tool: ChatToolCall = {
      kind: "toolCall",
      id: "recursive-list",
      title: "Get-ChildItem -Depth 2 | ForEach-Object { $rel }",
      toolKind: "execute",
      status: "completed",
      content: [{ type: "content", content: { type: "text", text: output } }],
      locations: [{ path: "C:/repo" }],
      rawInput: { command: "Get-ChildItem -Depth 2", cwd: "C:/repo" },
      createdAt: 1,
      updatedAt: 1,
    };
    // A nested directory has no extension, so the file heuristics reject it;
    // without the listing rule only the bare top level would be indexed, and
    // `.claude` would additionally be guessed as a dotfile.
    expect(collectToolOutputArtifacts(tool)).toEqual({
      files: [],
      directories: [],
      unknown: [
        "C:/repo/.claude",
        "C:/repo/.claude/commands",
        "C:/repo/.claude/commands/opsx",
        "C:/repo/docs",
        "C:/repo/docs/superpowers",
        "C:/repo/main.py",
      ],
    });
  });

  it("reads a PSIsContainer Name listing in either column order", () => {
    const output = [
      "PSIsContainer Name",
      "------------- ----",
      "         True docs",
      "        False README.md",
    ].join(String.fromCharCode(10));
    expect(extractArtifactDirectoriesFromText(output)).toEqual(["docs"]);
    expect(extractArtifactPathsFromText(output)).toEqual(["README.md"]);
  });

  it("splits aligned multi-column name listings into unresolved entries", () => {
    const tool: ChatToolCall = {
      kind: "toolCall",
      id: "wide-list",
      title: "Get-ChildItem -Name",
      toolKind: "execute",
      status: "completed",
      content: [
        {
          type: "content",
          content: {
            type: "text",
            text: ".github        .husky       packages\ninstall        LICENSE      README.md",
          },
        },
      ],
      locations: [],
      rawInput: { command: "Get-ChildItem -Name" },
      createdAt: 1,
      updatedAt: 1,
    };
    expect(collectToolOutputArtifacts(tool)).toEqual({
      files: [],
      directories: [],
      unknown: [
        ".github",
        ".husky",
        "packages",
        "install",
        "LICENSE",
        "README.md",
      ],
    });
  });

  it("qualifies nested listing entries with their owning directory", () => {
    const tool: ChatToolCall = {
      kind: "toolCall",
      id: "nested-list",
      title: "Get-ChildItem src -Name",
      toolKind: "execute",
      status: "completed",
      content: [
        {
          type: "content",
          content: { type: "text", text: "main.rs\ngenerated/" },
        },
      ],
      locations: [{ path: "C:/repo" }],
      rawInput: { command: "Get-ChildItem src -Name", cwd: "C:/repo" },
      createdAt: 1,
      updatedAt: 1,
    };
    expect(collectToolOutputArtifacts(tool)).toEqual({
      files: [],
      directories: ["C:/repo/src/generated"],
      unknown: ["C:/repo/src/main.rs"],
    });
  });
});

describe("collectToolOutputPaths", () => {
  it("reads filename arrays from search rawOutput", () => {
    const tool: ChatToolCall = {
      kind: "toolCall",
      id: "glob-1",
      title: "Glob",
      toolKind: "search",
      status: "completed",
      content: [],
      locations: [],
      rawOutput: { filenames: ["README.md", "docs/guide.md"] },
      createdAt: 1,
      updatedAt: 1,
    };
    expect(collectToolOutputPaths(tool)).toEqual([
      "README.md",
      "docs/guide.md",
    ]);
  });

  it("reads path arrays stored under singular provider keys", () => {
    const tool: ChatToolCall = {
      kind: "toolCall",
      id: "glob-2",
      title: "Glob",
      toolKind: "search",
      status: "completed",
      content: [],
      locations: [],
      rawOutput: { file: ["README.md", "docs/guide.md"] },
      createdAt: 1,
      updatedAt: 1,
    };
    expect(collectToolOutputPaths(tool)).toEqual([
      "README.md",
      "docs/guide.md",
    ]);
  });
});

describe("isPlainPathList", () => {
  it("accepts text fences that are only file paths", () => {
    expect(isPlainPathList("README.md\ndocs/guide.md", "text")).toBe(true);
    expect(isPlainPathList("fn main() {}", "rust")).toBe(false);
    expect(isPlainPathList("cargo test\nREADME.md", "text")).toBe(false);
    expect(isPlainPathList("PASS\nFAIL", "text")).toBe(false);
  });
});

describe("stripListMarker", () => {
  it("strips bullets, numbers, and task boxes", () => {
    expect(stripListMarker("- README.md")).toBe("README.md");
    expect(stripListMarker("1. docs/guide.md")).toBe("docs/guide.md");
    expect(stripListMarker("[ ] crates/engine/README.md")).toBe(
      "crates/engine/README.md",
    );
    expect(stripListMarker("├── cli")).toBe("cli");
    expect(stripListMarker("│   └── LICENSE")).toBe("LICENSE");
  });
});
