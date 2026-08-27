# Declare Hook contributions in config.json

Hook Plugins declare their immutable, versioned Hook descriptor in `assets/config.json` rather than a manifest `[hook]` section. This mirrors the MCP separation between distribution identity in `orax.toml` and contribution metadata in a strict configuration file, while allowing future plugin-global Settings to share the same declaration without duplicating Hook metadata.

## Considered Options

- A manifest `[hook]` section would be sufficient for the first RTK package but would separate future Setting declarations from the contribution that interprets them.
- Duplicating Hook fields between the manifest and configuration file would create two authorities.

## Consequences

- Every Hook Plugin must ship a Hook-shaped `assets/config.json`; RTK v0.1.0 omits the optional `settings` member.
- Settings-only, MCP `transport`, and Hook `hook` shapes are mutually exclusive and compile to distinct enum variants.
- `orax.toml` remains responsible for package identity, kind, version, release selection, and installed artifact target.
