/**
 * Picks a Cursor/VS Code-style unique name when pasting into a folder that already
 * contains the original basename: `name`, then `name copy`, then `name copy 2`.
 */
export function uniqueCopyName(
  fileName: string,
  occupied: ReadonlySet<string>,
): string {
  if (!occupied.has(fileName)) return fileName;
  const lastDot = fileName.lastIndexOf(".");
  const split = lastDot > 0;
  const stem = split ? fileName.slice(0, lastDot) : fileName;
  const ext = split ? fileName.slice(lastDot) : "";
  const first = `${stem} copy${ext}`;
  if (!occupied.has(first)) return first;
  for (let n = 2; n < Number.MAX_SAFE_INTEGER; n += 1) {
    const candidate = `${stem} copy ${n}${ext}`;
    if (!occupied.has(candidate)) return candidate;
  }
  return `${stem} copy${ext}`;
}

/** True when `child` is `parent` or a nested path under it. */
export function isWorkspacePathInside(parent: string, child: string): boolean {
  if (parent === "") return true;
  return child === parent || child.startsWith(`${parent}/`);
}
