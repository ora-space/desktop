import { spawnSync } from "node:child_process";

const gitCommands = [
  ["diff", "--name-only", "--diff-filter=ACMR"],
  ["diff", "--cached", "--name-only", "--diff-filter=ACMR"],
  ["ls-files", "--others", "--exclude-standard"],
];
const changedFiles = new Set();

for (const args of gitCommands) {
  const result = spawnSync("git", args, { encoding: "utf8" });
  if (result.status !== 0) process.exit(result.status ?? 1);
  for (const file of result.stdout.split("\n").filter(Boolean))
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
  const extension = file.split(".").pop();
  if (prettierExtensions.has(extension)) prettierFiles.push(file);
  if (extension === "rs") rustFiles.push(file);
}

if (prettierFiles.length > 0) {
  const pnpmCommand = process.platform === "win32" ? "pnpm.cmd" : "pnpm";
  const pnpmArgs = [
    "exec",
    "prettier",
    "--write",
    "--ignore-unknown",
    "--ignore-path",
    ".prettierignore",
    ...prettierFiles,
  ];
  const spawnCommand =
    process.platform === "win32"
      ? (process.env.ComSpec ?? "cmd.exe")
      : pnpmCommand;
  const spawnArgs =
    process.platform === "win32"
      ? ["/d", "/c", pnpmCommand, ...pnpmArgs]
      : pnpmArgs;
  const result = spawnSync(spawnCommand, spawnArgs, { stdio: "inherit" });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

for (const file of rustFiles) {
  const result = spawnSync("rustfmt", ["--edition", "2024", file], {
    stdio: "inherit",
  });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
