# Hook Configuration

Compiles the immutable, strongly typed `assets/config.json` Hook contribution of a `hook`-kind
Ora plugin package.

## Responsibilities

- Parses the strict `assets/config.json` Hook shape: `schemaVersion`, a required `hook` descriptor,
  and an optional Settings subset reserved for future plugin-global configuration.
- Compiles the closed, versioned `HookProtocol` enum so protocol semantics are explicit rather
  than open `Map<String, JsonValue>` metadata.
- Validates the package-relative executable path, the normalized bare `HookCommand` alias, and the
  embedded tool SemVer independently from the Hook Plugin version.

## Non-responsibilities

- Does not execute the declared executable. Runnability is established by release CI and isolated
  E2E tests, never during installation.
- Does not resolve the Hook against a running Agent Plugin. `ResolvedHook` and Agent-specific
  configuration belong to a future Agent Plugin consumption milestone.
- Does not own filesystem containment. The package validator that knows the package root re-checks
  that the executable resolves to a regular non-symlink file under `assets/`.

## Public boundary

- `CompiledHookConfiguration`: the validated, install-time descriptor returned to the package
  validator. It proves the declaration is legal, not that Settings are filled or that the
  executable starts.
- `HookProtocol`: the closed set of supported protocols. The first variant is `RtkRewriteV1`.
- `HookCommand`: the normalized bare command alias used for cross-Hook collision detection.

## Key invariants

- Settings-only, MCP `transport`, and Hook `hook` shapes are mutually exclusive; a file declaring
  `transport` and `hook` together fails closed (`MixedContribution`).
- Unknown root fields, unknown protocol variants, unknown descriptor fields, unsupported schema
  versions, and empty Settings all fail closed.
- The executable must be a portable relative path under `assets/`; the package validator enforces
  the actual filesystem containment.
- The command alias must be a bare name with no path separators so PATH resolution stays
  deterministic across installed Hooks.

## Failure semantics

Every compile error is a typed variant carrying a precise field path (`hook.executable`,
`hook.command`, `hook.toolVersion`) so malformed packages can be corrected efficiently.
