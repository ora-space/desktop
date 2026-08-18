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
  const extension = file.split(".").pop();
  if (!prettierExtensions.has(extension) && extension !== "rs") continue;

  const source = spawnSync("git", ["show", `:${file}`], { encoding: "buffer" });
  if (source.status !== 0) process.exit(source.status ?? 1);

  const command =
    extension === "rs"
      ? "rustfmt"
      : process.platform === "win32"
        ? "pnpm.cmd"
        : "pnpm";
  const args =
    extension === "rs"
      ? ["--edition", "2024", "--emit", "stdout"]
      : [
          "exec",
          "prettier",
          "--stdin-filepath",
          file,
          "--ignore-path",
          ".prettierignore",
        ];
  const spawnCommand =
    process.platform === "win32" ? (process.env.ComSpec ?? "cmd.exe") : command;
  const spawnArgs =
    process.platform === "win32" ? ["/d", "/c", command, ...args] : args;
  const formatted = spawnSync(spawnCommand, spawnArgs, {
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
