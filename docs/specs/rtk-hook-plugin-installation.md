## Problem Statement

Ora cannot currently represent, validate, install, or display a Hook Plugin. RTK therefore cannot be distributed through the Ora marketplace as a self-contained Hook: existing hosts reject the unknown kind, release selection cannot choose a native binary by target, and the package manager has no Hook-specific contribution or validation rules.

The first milestone must establish a trustworthy installation loop without taking ownership of Agent-specific configuration. A user must be able to discover the RTK Hook Plugin, download the correct Windows x86_64 artifact, verify and install it, see it available, and uninstall it. The installed package must contain a real, runnable RTK binary, while installation itself must never execute downloaded code.

## Solution

Add a processless `hook` Plugin Kind whose package contributes one immutable, strongly typed Hook Configuration and one package-contained executable. Complete the existing resolver-one target-artifact design so the marketplace can advertise target-specific releases and the host can reject unsupported targets before download.

Publish `official/rtk-ai.rtk` version `0.1.0` for `x86_64-pc-windows-msvc`, embedding RTK `0.45.0`. The Hook Configuration uses protocol `rtk-rewrite-v1`, command alias `rtk`, and the package-contained RTK executable. It contains no user Settings in the first release.

Install Hook Plugins with static validation only. Every installed valid Hook is available; the host has no enable/disable eligibility. If another installed Hook owns the same command alias, retain both installations, return `installed_with_command_conflict` with the colliding identity, and leave uniqueness to a future consumer. Establish payload runnability in the packaging workflow and isolated end-to-end tests, not inside the installer.

## User Stories

1. As an Ora user, I want RTK to appear in the plugin marketplace, so that I can discover it without installing RTK separately.
2. As an Ora user on Windows x86_64, I want the marketplace to identify RTK as compatible, so that I know I can install it.
3. As an Ora user on an unsupported target, I want RTK to remain visible but clearly marked incompatible, so that I understand why installation is unavailable.
4. As an Ora user, I want Ora to download only the artifact for my exact target, so that I do not receive unrelated native binaries.
5. As an Ora user, I want the downloaded archive verified against its marketplace SHA-256, so that corrupted or substituted content is rejected.
6. As an Ora user, I want a locally imported Hook archive to prove its target independently of marketplace metadata, so that local import applies the same compatibility boundary.
7. As an Ora user, I want installation to reject an archive for the wrong architecture or ABI, so that an unusable native executable is never installed as valid.
8. As an Ora user, I want the RTK executable contained inside the installed package, so that the plugin does not depend on a separate global RTK installation.
9. As an Ora user, I want installation to avoid executing downloaded binaries, so that installing a plugin does not silently cross the code-execution boundary.
10. As an Ora user, I want RTK available immediately after a successful installation, so that I do not have a separate enable step.
11. As an Ora user, I want command-alias conflicts reported explicitly, so that PATH resolution can never silently select the wrong Hook.
12. As an Ora user, I want a conflicting Hook to remain installed and available, so that I can inspect it; command uniqueness is deferred to a future consumer rather than faked as disable.
13. As an Ora user, I want uninstalling to be how I stop using RTK, because the host has no separate disable state.
14. As an Ora user, I want uninstalling RTK to remove its installed package, so that Ora no longer treats it as available locally.
15. As an Ora user, I want the installed plugin details to identify the Hook kind, protocol, command, target, plugin version, and embedded RTK version, so that I can audit what was installed.
16. As an Ora user, I want RTK v0.1.0 to omit a Configure action, so that Ora does not offer Settings that have no runtime consumer.
17. As an Ora user, I want the marketplace documentation to disclose RTK's local tracking behavior, so that future execution does not surprise me with locally retained command data.
18. As a plugin author, I want Hook contribution metadata in a strict configuration shape, so that the host can validate it without executing code.
19. As a plugin author, I want Hook Configuration distinct from MCP transport and Settings-only configuration, so that invalid mixed contributions are unrepresentable.
20. As a plugin author, I want one Hook Plugin to contribute exactly one Hook, so that identity, versioning, and errors remain unambiguous.
21. As a plugin author, I want the Plugin version independent from the embedded tool version, so that packaging fixes do not misrepresent an RTK release.
22. As a plugin author, I want target-specific artifacts to share one canonical Plugin identity, so that adding platforms does not fragment marketplace presence.
23. As a plugin author, I want deterministic validation errors with precise field paths, so that malformed packages can be corrected efficiently.
24. As a plugin author, I want future Settings limited to declared typed values, so that arbitrary JSON cannot become an undocumented protocol extension.
25. As an Agent Plugin developer, I want `rtk-rewrite-v1` represented as a versioned protocol, so that a future integration can opt into an explicit contract.
26. As an Agent Plugin developer, I want Agent-specific configuration excluded from the Hook package, so that the Agent Plugin remains the authority on its Agent's configuration format.
27. As an Ora maintainer, I want Hook Plugins to be processless, so that installing one does not create an unnecessary Deno runtime.
28. As an Ora maintainer, I want target selection implemented as the existing resolver-one release alternative, so that universal and native plugins use one coherent marketplace model.
29. As an Ora maintainer, I want universal and target-indexed release sources modeled as mutually exclusive states, so that URL-selection precedence cannot be ambiguous.
30. As an Ora maintainer, I want the archive's Hook target canonicalized and contained inside its package, so that symlinks or path traversal cannot escape the immutable package root.
31. As an Ora maintainer, I want Hook command aliases normalized and collision-checked across installed Hooks, so that a future consumer can refuse ambiguous PATH resolution.
32. As an Ora maintainer, I want installation outcomes modeled as a closed enum, so that callers cannot observe contradictory success flags.
33. As an Ora maintainer, I want an invalid Hook Configuration rejected by kind-aware package validation, so that a package cannot masquerade as another contribution type.
34. As an Ora maintainer, I want marketplace support information cached in the registry index, so that the UI can disable installation before downloading an unsupported artifact.
35. As a release engineer, I want the packaging workflow to pin the upstream RTK tag, commit, and asset digest, so that the embedded binary has an auditable origin.
36. As a release engineer, I want the RTK license included in the Hook package, so that redistribution satisfies the upstream Apache-2.0 terms.
37. As a release engineer, I want the packaging workflow to test the exact produced archive on Windows, so that a successful release contains a runnable RTK executable.
38. As a release engineer, I want smoke tests to verify RTK version and rewrite exit/output behavior, so that the declared Hook Protocol matches the payload.
39. As a release engineer, I want a process-local PATH test for the installed Hook directory, so that RTK's bare self-invocation requirement is verified without changing the machine environment.
40. As a marketplace maintainer, I want the listing to reference an already-published and verified release asset, so that marketplace main never points to a missing package.
41. As a marketplace maintainer, I want the RTK listing submitted through a reviewable pull request, so that identity, URL, digest, README, and logo changes have an audit surface.
42. As a developer validating the milestone, I want a clean-data-directory marketplace journey, so that cached sources or prior installations cannot hide integration defects.

## Implementation Decisions

- Add `hook` as a closed Plugin Kind across manifest parsing, installed contributions, lifecycle projection, contracts, and UI projections.
- A Hook Plugin is processless, has no `main.js`, and contributes exactly one Hook.
- Keep resolver version one. Implement the already-specified release-source union: either one universal URL/digest pair or one or more unique exact-target artifacts.
- Use canonical Rust target triples from a known allowlist. The first RTK artifact supports only `x86_64-pc-windows-msvc`; target matching never falls back across architecture, operating system, libc, or ABI. Unknown triples such as `not-a-real-triple` fail closed.
- Require a targeted installed archive to carry an artifact target. Online installation must match it against the selected release target; local import must match it against the current host.
- Keep Hook contribution metadata out of `orax.toml`. Every Hook package must carry a Hook-shaped `assets/config.json` with schema version, protocol, package-relative executable, bare command alias, and embedded tool version.
- Extend configuration compilation to a closed union of Settings-only, MCP, and Hook shapes. A Hook shape and MCP transport may not coexist. Unknown fields and unsupported protocol versions fail closed.
- Permit Hook Configuration to declare optional plugin-global Settings in future, but the first RTK package declares none.
- Restrict future custom Settings to declared `string`, `number`, and `boolean` values supported by the existing configuration system. Do not introduce arbitrary JSON, Agent templates, arrays encoded as strings, paths, or secrets in this milestone.
- Compile `rtk-rewrite-v1` into a protocol-specific descriptor rather than storing an open string-to-JSON metadata map.
- Require the executable to resolve to a non-symlink regular file contained under the package assets tree. The Windows milestone also requires the `.exe` suffix. Do not parse PE headers in this milestone.
- Validate a normalized bare command alias that cannot contain path separators. Command-alias collisions across installed Hooks are reported on the typed install outcome; uniqueness is deferred to a future consumer.
- Model the result as either `installed` or `installed_with_command_conflict` with the colliding Plugin identity. Both packages remain available; do not invent disable/eligibility state.
- Do not execute the Hook payload during marketplace sync, download, import, discovery, installation, or uninstall.
- Expose release compatibility in the registry projection so unsupported plugins remain visible while installation is disabled.
- Display the Hook protocol, target, command alias, Plugin version, and embedded tool version in installed details. Do not show Configure when the Hook Configuration has no Settings declaration.
- Publish the wrapper as `official/rtk-ai.rtk`, Plugin version `0.1.0`, embedding RTK `0.45.0`.
- Build and host release artifacts in a dedicated `ora-space/rtk-hook-plugin` repository. The marketplace repository remains a listing and documentation source rather than the binary build owner.
- Package the immutable manifest, Hook Configuration, RTK executable, upstream license, user-facing README, and safe SVG logo. Do not package RTK source, Rust toolchains, or `main.js`.
- Pin and verify the upstream RTK release asset before packaging, then compute and publish a separate digest for the final `.orax`.
- Document that RTK `0.45.0` cannot truly disable tracking. Future consumers must redirect its database to Ora-managed data, disable tee and telemetry, and disclose the fixed local retention behavior.
- Do not declare an Ora host-version dependency in the first RTK listing. Fixing canonical host-version injection and enforcing existing Ora dependency metadata is a separate product-wide change.
- Publish in dependency order: complete compatible Ora support, publish the wrapper release, open the marketplace PR, validate against the PR source with a compatible Ora build, then merge the marketplace listing.
- Use the repository glossary terms Hook Plugin, Hook Configuration, Hook Protocol, Hook Command, Hook Target, Targeted Artifact, Installed Hook Plugin, and Runnable Hook Package.

## Testing Decisions

- Prefer one high-level backend installation seam that exercises registry manifest parsing, target selection, injected download, digest verification, archive extraction, installed-manifest parsing, Hook Configuration compilation, filesystem containment, discovery, command conflict handling, and uninstall against a temporary Ora data directory.
- Follow the existing MCP marketplace-install and local-import backend tests as prior art. Reuse the real package manager and lifecycle while substituting only external network download and host target inputs.
- Assert external outcomes: installed files and identity, typed installation outcome, availability, incompatibility reason, conflict identity, and final uninstall state. Avoid tests that merely mirror internal helper calls or private fields.
- Add manifest-level tests for the universal/targeted release union, duplicate targets, unsupported triples, missing artifact target, and release/installed form separation.
- Add configuration-level tests for Settings-only/MCP/Hook exclusivity, strict unknown-field rejection, protocol versioning, executable path grammar, command grammar, and embedded tool SemVer.
- Add manager tests for missing configuration, forbidden `main.js`, package escapes, symlinks, non-files, wrong target, wrong extension, and kind/configuration mismatch.
- Add backend tests for both installation outcomes: `installed` and `installed_with_command_conflict` because of a command conflict. The conflict test must not require `RTK_RELEASE_ORAX`.
- Add focused UI tests for compatible and incompatible marketplace entries, disabled install action, Hook installed details, typed installation outcomes, and the absence of Configure when no Settings exist.
- Test the produced release artifact on a Windows runner. Extract it into an installed-style directory and execute the declared absolute executable to verify RTK `0.45.0` and the `rtk-rewrite-v1` stdout/exit-code contract.
- Add a process-scoped PATH smoke test that prepends only the extracted Hook directory and resolves bare `rtk`. Never modify user, system, or Ora-global environment state.
- Redirect RTK's database to a temporary directory and disable tee and telemetry during executable tests, so tests do not retain command history or emit network telemetry.
- Complete the milestone with a real clean-data-directory E2E using a compatible Windows x86_64 Ora build and the marketplace PR branch, covering sync, compatible display, download, SHA verification, install, discovery, and uninstall.
- Run the repository formatter and full lint/test task before completion, including frontend clean-stderr enforcement.

## Out of Scope

- Configuring RTK for any specific Agent.
- Delivering installed Hooks or Stored Setting Values to Agent Plugins.
- `ResolvedHook`, Agent SDK Hook APIs, Agent start parameters, or Agent-specific configuration templates.
- Workspace or project-level Hook selection; availability is application-global.
- Injecting Hook directories into real Agent process PATH values.
- Executing RTK as part of plugin installation.
- User-configurable RTK Settings in Plugin version `0.1.0`.
- Secret, file, directory, enum, list, map, or arbitrary JSON Setting types.
- Fixing RTK's ineffective tracking-disable configuration.
- Enforcing `[dependencies].ora` or correcting Ora's canonical host-version source.
- macOS, Linux, Windows ARM64, or other RTK target artifacts in the first release.
- Unix executable-mode preservation in archive extraction.
- Command-alias uniqueness at the host: both colliding packages remain available; a future consumer must refuse ambiguous PATH resolution.
- Consumer reconciliation for Hook uninstall or upgrade. Before Hook consumption ships, a later design must require affected Agents to become idle and restart before an in-use payload is removed.
- General plugin signing, SLSA provenance, revocation, or vulnerability advisories beyond existing HTTPS and SHA-256 verification.

## Further Notes

- RTK rewrites commands to invoke a bare `rtk` command. The packaging smoke test proves that a process-local PATH projection works, but production Agent PATH construction belongs to the future Agent Plugin integration milestone.
- RTK `0.45.0` stores local tracking data and does not honor its declared tracking-disable setting. The Hook is inert in this installation-only milestone, but marketplace documentation must disclose the behavior before a future consumer executes it.
- Hook Settings, if introduced later, are plugin-global and version-independent under the existing configuration service. They do not imply Workspace selection or Agent materialization.
- Existing processless plugins report a `stopped` runtime state. Every installed plugin is available; there is no enable/disable eligibility.
- The accepted testing seams were confirmed during design: backend installation as the primary seam, release-artifact execution as the payload seam, and a clean marketplace journey as the final system seam.
