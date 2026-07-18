import { rmSync } from "node:fs";

rmSync(new URL("../.test-dist/", import.meta.url), { force: true, recursive: true });
