# ora-utils::archive

Safe materialization of untrusted archives and folder trees. Enabled with the `archive` Cargo
feature.

## Responsibilities

- `ArchiveFormat` derives the container format from a file extension; extraction re-validates
  the content signature and rejects mismatches or corrupt structures before writing anything.
- `extract_archive` extracts `.zip` (including `.skill`) and `.tar.gz` / `.tgz` archives, and
  `copy_tree` materializes a local folder, both into a caller-owned destination directory and both
  returning an `ExtractedTree` listing of validated files.
- `ExtractLimits` bounds the raw archive size, cumulative extracted bytes, entry count, and entry
  path shape (`RelativePathLimits`).

## Key invariants

- Every entry path is a validated `StrictRelativePath` before it is written; zip-slip, traversal,
  absolute, and platform-unsafe names reject the whole tree.
- Encrypted archives, symlinks, hard links, devices, FIFOs, and sockets reject the whole tree.
  Folder copies skip symlinks and reject other special files.
- Paths that collide after NFC case folding reject the whole tree so the result is portable to
  case-insensitive filesystems.
- Archives are bounded by an expansion budget of `min(max_total_bytes, max(10 MiB, size * 100))`;
  folder copies are bounded by the flat `max_total_bytes`.
- Any failure aborts the whole tree; callers own cleanup of the destination directory.
