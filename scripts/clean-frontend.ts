import { fileURLToPath } from "node:url";
import path from "node:path";
import { lstat, readFile, realpath, rm } from "node:fs/promises";

const root = fileURLToPath(new URL("..", import.meta.url));
// The link check below compares a resolved member against the checkout, so the checkout has to
// be resolved too. A repository is routinely reached through a path `realpath` rewrites — a
// symlinked parent directory, or on Windows an 8.3 short name such as `C:\Users\RUIHAO~1\…` for
// a profile whose real name contains a space. Comparing a resolved member against an unresolved
// root then makes every member, including `.`, read as an escape from the checkout.
const resolvedRoot = await realpath(root);
const { workspace } = JSON.parse(
  await readFile(path.join(root, "deno.json"), "utf8"),
);

// Only remove dependency directories owned by explicit workspace members. This
// also works after installation has been removed and on Windows without a shell.
for (const member of [".", ...workspace]) {
  const directory = path.resolve(root, member);
  const relative = path.relative(root, directory);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error(`Workspace member is outside the repository: ${member}`);
  }
  const target = path.join(directory, "node_modules");
  try {
    // A linked workspace directory must not redirect cleanup outside this
    // checkout; a node_modules symlink itself is safe to unlink below.
    const resolvedRelative = path.relative(
      resolvedRoot,
      await realpath(directory),
    );
    if (
      resolvedRelative.startsWith("..") ||
      path.isAbsolute(resolvedRelative)
    ) {
      throw new Error(
        `Workspace member resolves outside the repository: ${member}`,
      );
    }
    const stat = await lstat(target);
    await rm(target, { recursive: !stat.isSymbolicLink(), force: true });
  } catch (error) {
    if (!(error instanceof Error && "code" in error && error.code === "ENOENT"))
      throw error;
  }
}
