# Ora Desktop

`ora-desktop` is the native Tauri host for Ora. It bootstraps the shared backend, exposes desktop-only commands to the frontend, owns native windows and dialogs, and adapts operating-system capabilities such as filesystem handoff and marketplace WebViews.

The crate does not own domain persistence or agent execution semantics; those remain in the shared backend and contract crates. Desktop commands translate between Tauri IPC and those stable boundaries.

Native marketplace windows use isolated browser profiles and provider-specific navigation policies. Their download events are routed into Ora-owned application data before the frontend is notified.

On Windows, `build.rs` omits Tauri's resource-embedded app manifest and instead
attaches the Common-Controls v6 side-by-side dependency via the linker for every
artifact (including `cargo test` harnesses). Without that, the lib-test binary binds
legacy comctl32 and fails to load with `STATUS_ENTRYPOINT_NOT_FOUND`.
