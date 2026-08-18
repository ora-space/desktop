# ora-skill-package

Reads and validates skill packages before they reach the application layer.

## Responsibilities and boundaries

- Materializes a validated snapshot of a skill source into OS temporary storage through
  `ora-utils::archive`. A source is either a folder tree or one supported archive (`.zip`,
  `.skill`, `.tar.gz`, `.tgz`), selected by the caller as a `SkillSource`.
- Owns the skill-level resource limits layered on top of the shared extraction limits: maximum
  discoverable skills, files per skill, and manifest bytes.
- Scans a snapshot for exact `SKILL.md` files and computes non-overlapping skill boundaries using
  the nearest-manifest-ownership rule.
- Parses and validates the YAML front matter of a `SKILL.md` manifest, returning structured
  candidate-level errors.

## Non-responsibilities

This crate does not implement archive or path safety itself; zip-slip defenses, encrypted and
special-entry rejection, portable case-conflict detection, path limits, and entry/byte budgets
are `ora-utils::archive` and `ora-utils::path` guarantees. It does not persist database records,
does not own import session lifecycle or timing, does not write into the formal skill directory
tree, and does not decide HTTP or IPC transport semantics.

## Key invariants

- Snapshot paths are `StrictRelativePath` values from `ora-utils`; every file in an
  `ExtractedTree` already passed validation before it was written.
- Resource-limit and path-safety failures reject the whole source as an `ArchiveError`; a
  malformed manifest only invalidates that one candidate and is surfaced as a `ManifestError`.
- Skill boundaries are computed only from files whose exact name is `SKILL.md`, and every file
  belongs to the deepest ancestor directory that owns a manifest.
