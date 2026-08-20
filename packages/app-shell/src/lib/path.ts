/**
 * Returns the directory containing an absolute host path, accepting both
 * separators because Desktop reports Windows and POSIX paths verbatim.
 *
 * A path without a separator (or with only a leading one) is returned as-is so
 * callers never hand an empty string to the host file manager.
 */
export function parentDirectory(path: string): string {
  const lastSeparator = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return lastSeparator > 0 ? path.slice(0, lastSeparator) : path;
}
