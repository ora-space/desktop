import { readdirSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../.test-dist/tests/", import.meta.url));

function collectTests(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      return collectTests(path);
    }
    return /\.(test|spec)\.js$/u.test(entry.name) ? [path] : [];
  });
}

const tests = collectTests(root);
if (tests.length === 0) {
  throw new Error("@ora-space/plugin-runtime matched zero test files");
}

console.log(`@ora-space/plugin-runtime: running ${tests.length} test files`);
const result = spawnSync(process.execPath, ["--test", ...tests], { stdio: "inherit" });
process.exit(result.status ?? 1);
