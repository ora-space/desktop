# Hook Configuration

Compiles the immutable, strongly typed `assets/config.json` Hook contribution of a `hook`-kind
Ora plugin package.

## Responsibilities

- Parses the strict `assets/config.json` Hook shape: `schemaVersion`, a required `hook` descriptor,
  and an optional Settings subset reserved for future plugin-global configuration.
- Validates the package-relative `executable` path, the optional `supportedAgents` display list,
  and the fixed arguments of both lifecycle phases.
- Reports members of the replaced declaration shape (`protocol`, `command`, `toolVersion`) by name
  so an author repackages instead of reading a generic unknown-field list.

## Non-responsibilities

- Does not execute the declared executable, and does not own the decision to execute it. Whether a
  lifecycle command runs is an authorization and backend concern.
- Does not resolve the Hook against a running Agent. Which Agents exist on this machine, and what
  they accept, is knowledge the tool holds and the host deliberately does not model.
- Does not own filesystem containment. The package validator that knows the package root re-checks
  that `executable` resolves to a regular non-symlink file under `assets/`, and the executor
  re-checks it again before every spawn.

## Public boundary

- `CompiledHookConfiguration`: the validated, install-time descriptor returned to the package
  validator. It proves the declaration is legal, not that Settings are filled or that the
  executable can run.
- `HookDescriptor`: the executable path, the advertised Agent identifiers, and the lifecycle.
- `HookLifecycle` / `HookLifecycleCommand`: `init` is mandatory, `deinit` is optional, and both
  carry the exact argument list the host passes verbatim.

## Key invariants

- Settings-only, MCP `transport`, and Hook `hook` shapes are mutually exclusive; a file declaring
  `transport` and `hook` together fails closed (`MixedContribution`).
- Unknown root fields, unknown descriptor fields, unsupported schema versions, and empty Settings
  all fail closed.
- The executable must be a portable relative path under `assets/`; the package validator and the
  executor enforce the actual filesystem containment.
- Lifecycle arguments are bounded in count and length and reject empty or control-bearing values,
  because they are handed to the operating system without a shell and echoed into diagnostics.
- `supportedAgents` is display information only: the compiler checks each identifier's shape and
  never matches it against Agents the host knows about.
