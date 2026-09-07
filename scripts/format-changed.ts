import { spawnSync } from "node:child_process";

const gitCommands = [
  ["diff", "-z", "--name-only", "--diff-filter=ACMR"],
  ["diff", "-z", "--cached", "--name-only", "--diff-filter=ACMR"],
  ["ls-files", "-z", "--others", "--exclude-standard"],
];
const changedFiles = new Set<string>();

for (const args of gitCommands) {
  const result = spawnSync("git", args, { encoding: "utf8" });
  if (result.status !== 0) process.exit(result.status ?? 1);
  for (const file of result.stdout.split("\0").filter(Boolean))
    changedFiles.add(file);
}

const prettierFiles = [];
const rustFiles = [];
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

for (const file of changedFiles) {
  const extension = file.split(".").pop() ?? "";
  if (prettierExtensions.has(extension)) prettierFiles.push(file);
  if (extension === "rs") rustFiles.push(file);
}

if (prettierFiles.length > 0) {
  const args = [
    "task",
    "--quiet",
    "format:files",
    "--write",
    "--ignore-unknown",
    "--ignore-path",
    ".prettierignore",
    ...prettierFiles,
  ];
  const result = spawnSync(Deno.execPath(), args, { stdio: "inherit" });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

for (const file of rustFiles) {
  const result = spawnSync("rustfmt", ["--edition", "2024", file], {
    stdio: "inherit",
  });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
