# Select target-specific Hook Plugin artifacts

One Hook Plugin version may publish separate artifacts for supported operating-system and processor-architecture targets. Ora selects one exact target before download instead of shipping a fat archive or giving each target a different plugin identity, so adding platforms does not change plugin identity and installations do not carry unrelated executables.

Release sources model universal and target-indexed artifacts as mutually exclusive alternatives. Every targeted archive repeats its target identity so local import and online installation apply the same host-compatibility check.

## Considered Options

- A fat archive would simplify registry metadata but waste bandwidth and disk space and enlarge the executable trust surface.
- Separate plugin identities per target would fragment versioning, enablement, and marketplace presentation.

## Consequences

- Release manifests and registry installation must distinguish target-specific artifacts from the target-neutral plugin identity.
- Installation must fail clearly when the current target has no artifact; the first RTK release intentionally publishes only Windows x86_64.
