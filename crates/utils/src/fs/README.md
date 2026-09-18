# ora-utils::fs

Portable file naming for files whose names come from untrusted sources such as browser download
suggestions, URL path segments, or user input. Atomic whole-file replacement lives in
`ora-utils::atomic`.

## Guarantees

- `sanitize_file_name(candidate, fallback_stem)` returns a single basename that is safe on every
  supported host: directory components are dropped, path separators, control characters, and
  Windows-illegal punctuation become `_`, leading and trailing spaces and dots are trimmed, the
  stem is capped at 120 bytes on a character boundary, and Windows reserved device names
  (`CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9`) are prefixed with `_`. The extension
  is preserved; an empty stem falls back to `fallback_stem`; a name without an extension gets none.
- `next_available_file_name(directory, file_name, occupied)` returns the first `stem.ext`,
  `stem-1.ext`, `stem-2.ext`, ... inside `directory` that neither exists on disk nor is claimed by
  the caller-supplied `occupied` predicate. The predicate lets callers account for reservations
  that are not visible on disk yet (for example in-flight downloads).

- `refuse_final_link(options)` adds the platform flag (`O_NOFOLLOW`, `FILE_FLAG_OPEN_REPARSE_POINT`)
  that makes an open refuse to traverse a link at the final path component, so a hostile path is
  opened under its own name and the handle's metadata can be inspected afterwards.
- `ExclusiveFileLock::try_acquire(path)` takes a non-blocking exclusive advisory lock on a sidecar
  lock file (creating it when absent, never through a link) and releases it on drop or process
  exit. It serializes writers across processes; it locks a sidecar rather than the guarded data
  file because Windows range locks would also block concurrent readers of that file.
- `classify_line_tail(file)` reads only the final byte of a line-oriented file and reports whether
  it is empty, newline-terminated, or ends mid-line, so an appender can avoid joining its first
  record onto a fragment left by a crashed writer.

## Extension convention

Both helpers treat the text after the last `.` of the basename as the extension. Multi-part
extensions such as `.tar.gz` are therefore split as stem `pack.tar` plus extension `gz`, so a
collision produces `pack.tar-1.gz`. This keeps the rule predictable and free of an ever-growing
list of known compound extensions; the original name is still preserved when no collision occurs.
A leading-dot name such as `.bashrc` has no extension and its stem is `bashrc` after trimming.

## Non-responsibilities

- Validating relative paths or containment (see `ora-utils::path`).
- Creating directories, removing files, or validating where a write may land.
