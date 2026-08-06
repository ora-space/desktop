/** Derives a project name from either a Windows or POSIX directory path. */
export function projectNameFromPath(rootPath: string): string {
  const original = rootPath.trim();
  const withoutTrailingSeparators = original.replace(/[\\/]+$/gu, "");
  return withoutTrailingSeparators.split(/[\\/]/u).filter(Boolean).at(-1) ?? original;
}
