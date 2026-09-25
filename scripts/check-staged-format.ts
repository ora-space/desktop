import { spawnSync } from "node:child_process";

const stagedFilesResult = spawnSync(
  "git",
  ["diff", "--cached", "--name-only", "--diff-filter=ACMR", "-z"],
  {
    encoding: "utf8",
  },
);
if (stagedFilesResult.status !== 0) process.exit(stagedFilesResult.status ?? 1);

const stagedFiles = stagedFilesResult.stdout.split("\0").filter(Boolean);
const prettierExtensions = new Set([
  "css",
  "html",
  "js",
  "json",
  "jsx",
  "md",
  "mjs",
  "ts",
  "tsx",
  "yaml",
  "yml",
]);
const unformattedFiles = [];

for (const file of stagedFiles) {
  const extension = file.split(".").pop() ?? "";
  if (!prettierExtensions.has(extension) && extension !== "rs") continue;

  const source = spawnSync("git", ["show", `:${file}`], { encoding: "buffer" });
  if (source.status !== 0) process.exit(source.status ?? 1);
  // rustfmt skips @generated files (a marker in the first five lines) when given a path, but not
  // on stdin; mirror that rule so generated code such as prost output is checked by its generator.
  if (
    extension === "rs" &&
    source.stdout
      .toString("utf8")
      .split("\n", 5)
      .some((line) => line.includes("@generated"))
  ) {
    continue;
  }

  const command = extension === "rs" ? "rustfmt" : Deno.execPath();
  const args =
    extension === "rs"
      ? ["--edition", "2024", "--emit", "stdout"]
      : [
          "task",
          "--quiet",
          "format:files",
          "--stdin-filepath",
          file,
          "--ignore-path",
          ".prettierignore",
        ];
  const formatted = spawnSync(command, args, {
    input: source.stdout,
    encoding: "buffer",
  });
  if (formatted.status !== 0) process.exit(formatted.status ?? 1);

  if (!source.stdout.equals(formatted.stdout)) unformattedFiles.push(file);
}

if (unformattedFiles.length > 0) {
  console.error("The following staged files are not formatted:");
  for (const file of unformattedFiles) console.error(`  ${file}`);
  console.error(
    "Run 'task format', then stage the formatted files and commit again.",
  );
  process.exit(1);
}
