# Ora Desktop

`ora-desktop` is the native Tauri host for Ora. It bootstraps the shared backend, exposes desktop-only commands to the frontend, owns native windows and dialogs, and adapts operating-system capabilities such as filesystem handoff and marketplace WebViews.

The crate does not own domain persistence or agent execution semantics; those remain in the shared backend and contract crates. Desktop commands translate between Tauri IPC and those stable boundaries.

Native marketplace windows use isolated browser profiles and provider-specific navigation policies. Their download events are routed into Ora-owned application data before the frontend is notified.

Ripgrep and Deno are bundled as Tauri sidecars under `binaries/rg` and
`binaries/deno` for release builds. Their platform-specific executables are
downloaded by `scripts/setup-binary.mjs` during the desktop build and are
intentionally excluded from version control. The script accepts `deno` or `rg`
as an optional argument to install only that sidecar; without an argument it
installs both. The packaging workflow adds the sidecars to Tauri's configuration
in its checkout immediately before building;
the checked-in configuration keeps `externalBin` empty so `tauri dev` does not
depend on that directory.

`BundledBinaryPaths` stores the paths in `DesktopState`. Debug builds, including
development and tests, pass `rg` and `deno` as command names so the operating
system resolves them from `PATH`. Release builds resolve the platform-specific
sidecars next to the Tauri executable. The shared backend and `ora-fs` receive
the resolved ripgrep path, while Rust-owned Deno integrations receive the
resolved Deno path. If a release sidecar is missing, Desktop logs the failure
and stops before constructing the application state.

On Windows, `build.rs` omits Tauri's resource-embedded app manifest and instead
attaches the Common-Controls v6 side-by-side dependency via the linker for every
artifact (including `cargo test` harnesses). Without that, the lib-test binary binds
legacy comctl32 and fails to load with `STATUS_ENTRYPOINT_NOT_FOUND`.
