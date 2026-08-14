# Desktop Release Process

How Ora desktop installers are built and published.

## Build pipeline

`.github/workflows/desktop-build.yml` builds installers for Linux, macOS, and
Windows in parallel and always uploads them as workflow artifacts. Publishing a
GitHub Release is an explicit opt-in, never implied by merely running the
workflow.

Both CI and local builds go through the same entry point so their outputs stay
identical:

```sh
task build:desktop
```

This regenerates the TypeScript contracts (`task export-contracts`), builds the
frontend (`pnpm run build` via Tauri's `beforeBuildCommand`), then compiles and
bundles the Tauri app. Bundles land in `apps/desktop/src-tauri/target/release/bundle/`.

Bundle targets are pinned in `apps/desktop/src-tauri/tauri.conf.json`
(`bundle.targets`): `.dmg` (macOS), NSIS `.exe` (Windows), `.AppImage` and
`.deb` (Linux). The bundler skips targets that do not apply to the host OS, so
the same list serves all three platforms. Every target listed there is one we
intend to ship to users; add or remove entries deliberately.

## Version source of truth

`apps/desktop/src-tauri/tauri.conf.json` (`version`) is the single source of
truth — installer filenames and package metadata come from it. Two mirrors must
be updated in the same commit when it changes:

- `apps/desktop/src-tauri/Cargo.toml` (`package.version`)
- `apps/desktop/package.json` (`version`)

There is no automatic sync; the release commit updates all three together.

## Toolchain pinning

CI and local builds share the same toolchain definitions:

- Rust: `rust-toolchain.toml` at the repository root.
- Node: `engines.node` and `packageManager` (pnpm) in the root `package.json`.

## Cutting a release

1. Bump the version in the three files above in one commit, e.g.
   `chore(desktop): bump version to 0.2.0`, and merge it to `main`.
2. Either:
   - push a tag: `git tag v0.2.0 && git push origin v0.2.0`, or
   - trigger `desktop-build` from the Actions page with `release=true` and
     `tag=v0.2.0`.
3. CI builds the three platforms and creates a **draft** GitHub Release with
   all installers attached.
4. Download and smoke-test the installers, then publish the draft manually.

The draft step is deliberate: artifacts are currently unsigned (macOS
Gatekeeper and Windows SmartScreen will warn), so a human check gates every
release. Signing and notarization are tracked separately; the workflow has a
comment marking where signing secrets plug in.

## Verifying without releasing

Trigger `desktop-build` manually with `release` unchecked: the three platform
jobs run and upload installers as workflow artifacts only, which is the way to
validate packaging changes from a branch.
