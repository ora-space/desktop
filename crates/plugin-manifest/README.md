# ora-plugin-manifest

`ora-plugin-manifest` parses and validates the TOML manifest for one published Ora plugin release.
It accepts caller-provided text and returns an immutable domain object whose public types preserve
the schema's semantic invariants.

## Responsibilities and boundaries

- Reject malformed TOML, missing or unknown fields, unsupported resolver versions, and invalid
  field values with structured errors.
- Model plugin names, source categories, plugin kinds, HTTPS URLs, SHA-256 digests, optional source
  repository metadata, and optional Ora host version requirements as validated values.
- Preserve deterministic validation order so callers receive a stable first error.
- Reuse domain-free slug and Git branch-name validation from `ora-utils`.

## Non-responsibilities

- No filesystem access, fixed manifest filename, source-path diagnostics, or input-size policy.
- No network access, download, repository probing, or release checksum calculation.
- No plugin installation, discovery, execution, update selection, or integration with
  `ora-plugin-manager`.
- No serialization, source rewriting, comment preservation, or compatibility with older formats.

## Public boundary

`PluginManifest::parse(&str)` is the only manifest construction entrypoint. Manifest fields stay
private and are exposed through read-only accessors. Reusable validated value types additionally
implement `FromStr`; none of the APIs provide an unchecked constructor.
