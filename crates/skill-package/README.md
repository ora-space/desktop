# ora-skill-package

Reads and validates skill packages before they reach the application layer.

## Responsibilities and boundaries

- Materializes a validated snapshot of a skill source into OS temporary storage. A source is
  either a folder tree or one supported archive (`.zip`, `.skill`, `.tar.gz`, `.tgz`).
- Enforces all archive and path security rules: zip-slip defenses, portable Unicode/case
  conflict detection, path segment/depth limits, special-entry rejection, and encrypted-archive
  rejection.
- Enforces session resource limits: raw archive size, cumulative extracted bytes, entry counts,
  and the archive expansion-ratio budget.
- Scans a snapshot for exact `SKILL.md` files and computes non-overlapping skill boundaries using
  the nearest-manifest-ownership rule.
- Parses and validates the YAML front matter of a `SKILL.md` manifest, returning structured
  candidate-level errors.

## Non-responsibilities

This crate does not persist database records, does not own import session lifecycle or timing,
does not write into the formal skill directory tree, and does not decide HTTP or IPC transport
semantics. It only materializes and validates a source snapshot inside the caller-provided
destination directory.

## Key invariants

- Every relative path stored in a `RelativePath` is a safe, validated, `\`-normalized UTF-8 path
  with no empty, `.`, or `..` segments or Windows reserved device names. Portable filename safety
  is inherited from `ora-fs`; this crate adds package-specific depth and length limits.
- Resource-limit and path-safety failures reject the whole source; a malformed manifest only
  invalidates that one candidate and is surfaced as a `ManifestError`.
- Archive entries are never followed as links and never written before their paths pass
  validation.
