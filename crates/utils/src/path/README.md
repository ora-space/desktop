# ora-utils::path

Platform-independent path validation, containment, and lexical normalization shared by every
filesystem-facing crate.

## Guarantees

- `PortableRelativePath` gives wire and configuration paths platform-independent validation and a
  slash-separated identity. It treats both slash styles as separators, drops empty and `.`
  segments, and rejects parent traversal, rooted paths, Windows drive/UNC prefixes, reserved
  device names, and NUL bytes on every host.
- `StrictRelativePath` is the strict counterpart for untrusted archive and package entries: any
  irregular spelling (empty, `.`, or `..` segments, trailing separators, control characters) is
  rejected instead of normalized, and segment-length, total-length, and depth limits from
  `RelativePathLimits` apply. It reuses the portable parser for platform-specific filename safety
  and provides an NFC case-folded key for portable conflict detection.
- `CanonicalPathRoot` centralizes canonical root identity, existing-target resolution, absolute
  selection containment, and conversion back to portable relative paths. Roots and existing
  requested paths are canonicalized before containment checks, including static symlink escape
  protection. These path-based checks do not protect against a concurrently replaced symlink
  between validation and use.
- The lexical helpers (`normalize_absolute`, `normalize_relative`,
  `canonicalize_longest_existing_prefix`) support comparisons of paths that may not exist yet
  without touching the filesystem beyond the existing prefix.

## Boundaries

The two relative-path types are intentionally distinct; callers choose by trust level and must
not convert one into the other implicitly.
