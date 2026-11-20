# Sidecars

English | [中文](sidecars.zh.md)

Release builds of Desktop bundle Deno and ripgrep beside the executable, and the Node image ships Deno.
Both are verified against reviewed pins before use. The `ora-reaper` sidecar is built from the
checked-out source by `task build:desktop` instead.

## Verification

`scripts/setup-binary.ts` runs before the Desktop frontend build. It checks Deno and ripgrep in
`apps/desktop/src-tauri/binaries` against the selected release's executable SHA-256 and, for a native
target, runs `--version`. A stale, corrupt or unexecutable file is replaced only after a new download
passes every check; foreign-target executables are authenticated by digest without running them.

`scripts/sidecar-checksums.json` is the trust source: its keys bind exact official GitHub release URLs
to archive and executable SHA-256 values for Linux and Windows x64, plus Deno for Linux arm64.
The Node image installs Deno from the same pins.

- Each installation stages in its own temporary directory inside `binaries`, on the destination
  filesystem. The archive must match its pin before extraction; the extracted regular executable must
  match its pin and, natively, report the expected version before an atomic rename publishes it.
- Failures keep the previous binary and fail the build. Cleanup removes only that installation's staging
  directory, concurrent installers never share staging, and existing files and legacy `.extract-*`
  directories are neither reused nor deleted.
- Unknown releases or targets, missing pins and malformed digests fail closed. There is no checksum
  override or unverified fallback; `--force` bypasses cache reuse only.

`deno task setup:binary` accepts `deno` or `rg` to install one sidecar. Debug Desktop builds use `deno`
and `rg` from `PATH`, see [Desktop host](../desktop/desktop-host.md#bundled-sidecars).

## Updating a sidecar

1. Choose the official release. For Deno, update `.deno-version` and `package.json` `engines.deno`
   (`deno task check:toolchain` checks both); for ripgrep, update the version in setup and CI.
2. Read the release asset digests from the official repository's GitHub release API
   (`/repos/denoland/deno/releases/tags/v<VERSION>` or `/repos/BurntSushi/ripgrep/releases/tags/<VERSION>`)
   and compare them with the published checksum assets where available.
3. Download each supported archive, verify its SHA-256, extract it, hash the `deno`/`rg` executable
   (`.exe` on Windows) and update both hashes under the exact release URL.
4. Review the pin changes, run `deno task test:tooling`, exercise the real download with
   `deno task setup:binary --force`, and run the Desktop build CI on its supported platforms.
