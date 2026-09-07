import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, readFile } from "node:fs/promises";
import path from "node:path";

export interface SidecarChecksums {
  archiveSha256: string;
  executableSha256: string;
}

/** Requires a reviewed release pin; unknown versions never fall back to unchecked downloads. */
export async function readSidecarChecksums(
  root: string,
  url: string,
): Promise<SidecarChecksums> {
  const manifest = JSON.parse(
    await readFile(
      path.join(root, "scripts", "sidecar-checksums.json"),
      "utf8",
    ),
  );
  const checksums = manifest[url];
  if (
    !checksums ||
    !/^[a-f0-9]{64}$/.test(checksums.archiveSha256) ||
    !/^[a-f0-9]{64}$/.test(checksums.executableSha256)
  ) {
    throw new Error(
      `Missing trusted SHA-256 pin for ${url}. Update scripts/sidecar-checksums.json from the official release.`,
    );
  }
  return checksums;
}

/** Hashes regular files without loading an entire runtime into memory or following symlinks. */
export async function verifySidecarHash(
  file: string,
  expected: string,
): Promise<void> {
  if (!(await lstat(file)).isFile())
    throw new Error(`Expected a regular sidecar file: ${file}`);
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  if (hash.digest("hex") !== expected)
    throw new Error(`SHA-256 mismatch: ${file}`);
}
